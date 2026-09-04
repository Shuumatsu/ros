//! Physical and virtual memory management.

pub mod address_space;
pub mod boot_table;
pub mod direct_map;
pub mod kernel_table;
pub mod layout;
pub mod machine;
pub mod phys_range;
pub mod stack;
pub mod user;
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

/// Kernel translation scheme.
pub type KernelScheme = mmu::Sv39;

/// Initialize the frame allocator, then the heap.
///
/// Call on the boot hart. Boot-time stacks are allocated after this returns and before
/// [`init_page_table`].
pub fn init_allocators(machine: MachineMemory<'_>) {
    layout::report();
    layout::check();
    stack::check();
    direct_map::report();
    machine.check();

    let (image_start, image_end) = layout::image();
    let image =
        PhysRange::new("kernel image", virt_to_phys(image_start), image_end.sub_addr(image_start));
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

    heap::init();
    heap::self_test();
}

/// Build, audit, and activate the final kernel page table.
///
/// Call on the boot hart after [`init_allocators`] and all boot-time stack allocations.
/// Further [`stack::alloc`] calls panic.
pub fn init_page_table(machine: MachineMemory<'_>) {
    stack::report();
    kernel_va::report();

    // The table maps the stacks that exist now, so no later one could be mapped by anything.
    stack::seal();
    kernel_table::init(machine.mmio);

    stack::self_test(alloc_kernel_stack("runtime stack self-test"));
}

/// Allocate and map a runtime kernel stack.
///
/// The stack is immediately usable only on the calling hart.
///
/// # Panics
///
/// Panics if contiguous RAM is unavailable, the kernel table is not live, or auditing fails.
pub fn alloc_kernel_stack(name: &'static str) -> Stack {
    let stack = stack::reserve(name);
    kernel_table::map_stack(&stack);
    stack
}
