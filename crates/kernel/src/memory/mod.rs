//! Physical and virtual memory.
//!
//! One module per concern, and the two `init` functions below are the only places that
//! know the order they have to come up in:
//!
//! | module | owns |
//! |---|---|
//! | [`layout`] | the kernel image, as the linker laid it out |
//! | [`phys_range`] | a named physical range, and the geometry of a list of them |
//! | [`machine`] | what the kernel is told about physical memory, and its one seam |
//! | [`direct_map`] | `VA = PA + offset`, how far it reaches, and the conversions |
//! | [`boot_table`] | the compile-time table the architecture entry installs |
//! | [`frame`] | which physical frames the kernel has, and vending them |
//! | [`heap`] | the `#[global_allocator]`, carved out of those frames |
//! | [`kernel_va`] | which kernel virtual addresses are taken |
//! | [`stack`] | stack geometry, and every stack that must be mapped |
//! | [`address_space`] | a page-table tree, its root frame, and its `satp` |
//! | [`region`] | installing, auditing and reporting a list of mappings |
//! | [`kernel_table`] | *which* mappings the kernel gets, and switching to them |
//! | [`user_table`] | *which* mappings a user image gets, in a space of its own |
//!
//! Nothing here decides a fact one of those owns, and nothing here reads the device tree:
//! platform facts arrive through [`machine::MachineMemory`].
//!
//! # Naming
//!
//! Nothing is re-exported from this file. A caller writes `direct_map::phys_to_virt` and
//! `phys_range::PhysRange`, so the module that owns the fact is named at every use — which
//! for the conversions is the difference between "the kernel's PA-to-VA" and "add the
//! direct map's offset, which is only defined below [`direct_map::END`]". One spelling per
//! fact holds inside the subsystem too: a module here reaches a sibling as `super::name`,
//! never `crate::memory::name`, which is left to the nested modules that have no other way
//! to say it.
//!
//! Two visibilities, and they mean something: `pub` is a module the rest of the kernel has
//! business with, `pub(in crate::memory)` is one it does not. Widening the second kind is
//! a deliberate one-line change rather than the default.
//!
//! Each module states its own facts in one shape: `check` rejects a bad one before anything
//! derives an address from it, `report` prints it afterwards, and `self_test` exercises what
//! is worth exercising. Where a fact does not outlive `init` there is no `report` — the
//! kernel table's region list dies with the function that builds it, so it prints from
//! inside.

pub mod address_space;
pub mod boot_table;
pub mod direct_map;
pub mod kernel_table;
pub mod layout;
pub mod machine;
pub mod phys_range;
pub mod stack;
pub mod user_table;

pub(in crate::memory) mod frame;
pub(in crate::memory) mod heap;
pub(in crate::memory) mod kernel_va;
pub(in crate::memory) mod region;

use mmu::MemoryAddr;

use crate::utils::ByteSize;

use direct_map::virt_to_phys;
use machine::MachineMemory;
use phys_range::PhysRange;
use stack::Stack;

/// The translation scheme this kernel runs under, chosen here and nowhere else.
///
/// Every scheme-dependent fact below follows from it: how wide a canonical half is and so
/// how far [`direct_map`] reaches, what page sizes [`kernel_table`] may tile with, which
/// `satp.MODE` [`address_space`] and [`boot_table`] write. None of them names a scheme of
/// its own, so moving to Sv48 is this line plus `kernel.ld`'s `_va_offset` — which the
/// boot entry already reconciles against [`direct_map::VA_OFFSET`] before it jumps high.
pub type KernelScheme = mmu::Sv39;

/// Bring the allocators up: physical frames, then the heap carved out of them.
///
/// **Boot hart only** — a secondary's architecture entry installs the finished page table
/// and stack before it reaches Rust.
///
/// Everything from here to [`init_page_table`] is one sequence with a hole in the middle:
/// whoever needs an address of its own must take it before the table that maps it is built.
/// [`crate::cpu`] fills that hole with one [`stack::alloc`] per hart it means to start.
/// Splitting it here rather than calling into `cpu` is what keeps this subsystem free of
/// dependencies on its customers; [`crate::start`] owns the order across the two.
///
/// `machine` is a parameter rather than a lookup: it is whoever probed the platform's to
/// describe, and that discoverer already depends on this module.
pub fn init_allocators(machine: MachineMemory<'_>) {
    // Before any *mapping* is derived from the linker symbols or the machine. Addresses
    // have been derived already — `device_tree` located the kernel's RAM bank by its load
    // address, and the console reached the UART through the direct map — so these confirm
    // what the earlier boot stages assumed rather than gate the first use.
    layout::report();
    layout::check();
    stack::check();
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
}

/// Build the kernel's own page table and switch to it: per-section rights and W^X,
/// replacing the boot table's blanket RWX gigapages.
///
/// Call on the boot hart, after [`init_allocators`] and after every stack exists. From here
/// on [`stack::alloc`] refuses, since a stack this table does not map is one a hart faults
/// on rather than runs on.
pub fn init_page_table(machine: MachineMemory<'_>) {
    // Reported here rather than in `init_allocators`, because the stacks and the addresses
    // they were given do not exist until `cpu` has asked for them.
    stack::report();
    kernel_va::report();

    kernel_table::init(machine.mmio);

    // The runtime stack path, exercised where a failure is still one assertion rather than a
    // fault inside the first context switch that uses one.
    stack::self_test(alloc_kernel_stack("runtime stack self-test"));
}

/// Allocate a kernel stack and map it into the live kernel table.
///
/// For a stack whose owner does not exist when [`init_page_table`] runs — a process's kernel
/// stack, which lives and dies with the process. [`stack::alloc`] cannot serve it: that one is
/// mapped by the table build, which is over.
///
/// The two steps belong to two modules and the order belongs to neither, which is why the
/// composition is here: [`stack`] owns what a stack is and where it goes, [`kernel_table`] owns
/// what the kernel's table maps. Callers get a stack that is already usable.
///
/// **Usable on the calling hart only**, for [`kernel_table::map_stack`]'s reason.
///
/// # Panics
///
/// If there is no contiguous RAM for it, if the kernel table is not live yet, or if the mapping
/// does not audit.
pub fn alloc_kernel_stack(name: &'static str) -> Stack {
    let stack = stack::reserve(name);
    kernel_table::map_stack(&stack);
    stack
}
