//! Physical and virtual memory.
//!
//! One module per concern, and [`init`] is the only place that knows the order they
//! have to come up in:
//!
//! | module | owns |
//! |---|---|
//! | [`layout`] | the kernel image, as the linker laid it out |
//! | [`machine`] | what the kernel is told about physical memory, and its one seam |
//! | [`direct_map`] | `VA = PA + offset`, how far it reaches, and the conversions |
//! | [`boot_table`] | the compile-time table the architecture entry installs |
//! | [`frame`] | which physical frames the kernel has, and vending them |
//! | [`heap`] | the `#[global_allocator]`, carved out of those frames |
//! | [`kernel_va`] | which kernel virtual addresses are taken |
//! | [`stack`] | one stack per hart, and its guard page |
//! | [`address_space`] | an Sv39 tree, its root frame, and its `satp` |
//! | [`region`] | installing, auditing and reporting a list of mappings |
//! | [`kernel_table`] | *which* mappings the kernel gets, and switching to them |
//!
//! Nothing here decides a fact one of those owns, and nothing here reads the device tree:
//! platform facts arrive through [`machine::MachineMemory`].

pub mod address_space;
pub(crate) mod boot_table;
pub mod direct_map;
pub mod frame;
pub mod heap;
pub mod kernel_table;
pub mod kernel_va;
pub mod layout;
pub mod machine;
pub mod region;
pub mod stack;

use paging::MemoryAddr;

use crate::utils::ByteSize;

pub use direct_map::{phys_to_virt, virt_to_phys};
pub use machine::MachineMemory;

use machine::PhysRange;

/// Bring the memory subsystem up. **Boot hart only** — a secondary's architecture entry
/// installs the finished page table and stack before it reaches Rust.
///
/// The order is the content of this function, each step a customer of the one before:
/// frames (no heap of their own), then the heap carved out of them, then the secondary
/// stacks (frames *and* heap, and they must exist before the table that maps them), then
/// the page table.
///
/// Both arguments are parameters rather than lookups, for the same reason: `machine` is
/// whoever probed the platform's to describe, `secondary_harts` is `cpu`'s to decide, and
/// both of those already depend on this module. `start` knows all three.
pub fn init(machine: MachineMemory<'_>, secondary_harts: impl Iterator<Item = usize>) {
    // Before any *mapping* is derived from the linker symbols or the machine. Addresses
    // have been derived already — `device_tree` located the kernel's RAM bank by its load
    // address, and the console reached the UART through the direct map — so these confirm
    // what the earlier boot stages assumed rather than gate the first use.
    layout::report();
    layout::check();
    stack::check_layout();
    direct_map::report();
    machine.check();

    // 1. Physical frames: the whole RAM bank, with what is spoken for withheld from inside
    //    it. The machine describes all of that but one range — the image the kernel is
    //    running out of — which is this module's to add. Starting the pool above the image
    //    instead would be a second answer to "not ours" and the weaker one, since it says
    //    nothing about where the firmware's own memory ends.
    let image = PhysRange::new(
        "kernel image",
        virt_to_phys(layout::memory_start()),
        layout::kernel_top().sub_addr(layout::memory_start()),
    );
    assert!(
        machine.ram.base <= image.base && image.end() <= machine.ram.end(),
        "the kernel image occupies {:#x}..{:#x}, which is not inside the RAM bank at \
         {:#x}..{:#x} the machine says it was loaded into",
        image.base,
        image.end(),
        machine.ram.base,
        machine.ram.end()
    );
    println!(
        "[memory] RAM bank {:#x}..{:#x} ({}); the kernel image and everything the machine \
         reserved are withheld from it, and nothing else is",
        machine.ram.base,
        machine.ram.end(),
        ByteSize(machine.ram.size)
    );
    frame::init(machine.ram, image, machine.foreign);
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
    kernel_table::init(machine.mmio);
}
