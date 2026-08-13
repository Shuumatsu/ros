//! Physical and virtual memory.
//!
//! One module per concern, and [`init`] is the only place that knows the order they
//! have to come up in:
//!
//! | module | owns |
//! |---|---|
//! | [`layout`] | the kernel image, as the linker laid it out |
//! | [`direct_map`] | `VA = PA + offset`, and the conversions made of it |
//! | [`boot_table`] | the compile-time table the architecture entry installs |
//! | [`frame`] | which physical frames the kernel has, and vending them |
//! | [`heap`] | the `#[global_allocator]`, carved out of those frames |
//! | [`kernel_va`] | which kernel virtual addresses are taken |
//! | [`stack`] | one stack per hart, and its guard page |
//! | [`address_space`] | an Sv39 tree, its root frame, and its `satp` |
//! | [`region`] | installing, auditing and reporting a list of mappings |
//! | [`kernel_table`] | *which* mappings the kernel gets, and switching to them |
//!
//! Nothing here decides a fact one of those owns.

use paging::PhysicalAddr;

pub mod address_space;
pub(crate) mod boot_table;
pub mod direct_map;
pub mod frame;
pub mod heap;
pub mod kernel_table;
pub mod kernel_va;
pub mod layout;
pub mod region;
pub mod stack;

pub use direct_map::{phys_to_virt, virt_to_phys};

/// Bring the memory subsystem up. **Boot hart only** — a secondary's architecture entry
/// installs the finished page table and stack before it reaches Rust.
///
/// The order is the content of this function, each step a customer of the one before:
/// frames (no heap of their own), then the heap carved out of them, then the secondary
/// stacks (frames *and* heap, and they must exist before the table that maps them), then
/// the page table.
///
/// `secondary_harts` is a parameter because deciding which harts to start is `cpu`'s
/// business, and `cpu` already depends on this module; `start` knows both.
pub fn init(secondary_harts: impl Iterator<Item = usize>) {
    // Before anything derives an address from the linker symbols.
    layout::report();
    layout::check();
    stack::check_layout();
    direct_map::report();

    // 1. Physical frames: [free_start, ram_end). `free_start` is the top of the kernel
    //    image (a high VA); the allocator vends *physical* addresses, so convert it
    //    back. `ram_end` is already physical, validated by `device_tree::init`.
    let free_start_pa = virt_to_phys(layout::free_ram_start());
    let ram_end = PhysicalAddr::new(crate::device_tree::ram_end().expect(
        "device tree RAM region not discovered; call device_tree::init before memory::init",
    ));
    assert!(
        free_start_pa < ram_end,
        "kernel image top {free_start_pa:#x} meets/exceeds RAM top {ram_end:#x}; give the VM more RAM"
    );
    frame::init(free_start_pa, ram_end);
    frame::report();
    frame::self_test();

    // 2. The kernel heap, from the frames above.
    heap::init();
    heap::self_test();

    // 3. Secondary hart stacks, at addresses from `kernel_va`.
    stack::init(secondary_harts);
    stack::report();
    kernel_va::report();

    // 4. The real kernel page table: per-section rights and W^X, replacing the boot
    //    table's blanket RWX gigapages.
    kernel_table::init();
}
