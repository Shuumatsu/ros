use buddy_system_allocator::LockedHeap;
use core::alloc::Layout;

pub mod frame;
pub mod layout;
pub mod paging;

/// ORDER determines max allocation size: 2^(ORDER-1) bytes
/// ORDER=32 supports up to 2GB allocations
const ORDER: usize = 32;

#[global_allocator]
static HEAP: LockedHeap<ORDER> = LockedHeap::empty();

/// Bytes carved off the top of the kernel image for the kernel heap. Kept
/// bounded on purpose: the *rest* of RAM — the bulk — belongs to the physical
/// frame allocator (`frame`), which cannot exist until this heap does (it keeps
/// its free lists here). The heap holds only kernel bookkeeping, `frame`'s
/// `BTreeSet`s included; 8 MiB is ample. Prefer `frame` for anything page-sized;
/// grow this only if you must `Box` large buffers.
const KERNEL_HEAP_SIZE: usize = 8 * 1024 * 1024;

#[alloc_error_handler]
fn alloc_error(layout: Layout) -> ! {
    panic!(
        "Heap allocation failed: {} bytes with {}-byte alignment",
        layout.size(),
        layout.align()
    );
}

/// Initialize the memory subsystem: kernel heap first, then the physical frame
/// allocator over the rest of RAM.
///
/// Order is load-bearing. `frame` (buddy) keeps its free lists on the heap, so
/// the heap must exist before we add frames. Both regions are sized from RAM
/// discovered at runtime — the heap from the linker's `_heap_start` up by
/// [`KERNEL_HEAP_SIZE`], the frames from there to the device-tree RAM top — so
/// nothing here is a compile-time guess about how much RAM exists.
pub fn init() {
    // 1. Kernel heap: [_heap_start, _heap_start + KERNEL_HEAP_SIZE).
    let heap_start = layout::heap_start();
    let heap_end = heap_start + KERNEL_HEAP_SIZE;
    unsafe {
        HEAP.lock().add_to_heap(heap_start, heap_end);
    }
    println!(
        "[memory] heap:   {:#x}..{:#x} ({} MiB)",
        heap_start,
        heap_end,
        KERNEL_HEAP_SIZE / 1024 / 1024
    );

    // 2. Physical frames: [heap_end, ram_end). The RAM top comes from the device
    //    tree, which `device_tree::init` already validated (it panics on an
    //    unusable tree), so it is authoritative by the time we get here.
    let ram_end = crate::device_tree::ram_end()
        .expect("device tree RAM region not discovered; call device_tree::init before memory::init");
    assert!(
        heap_end < ram_end,
        "kernel heap top {heap_end:#x} meets/exceeds RAM top {ram_end:#x}; shrink KERNEL_HEAP_SIZE or give the VM more RAM"
    );
    frame::add_range(heap_end, ram_end);
    println!(
        "[memory] frames: {:#x}..{:#x} ({} MiB)",
        heap_end,
        ram_end,
        (ram_end - heap_end) / 1024 / 1024
    );

    frame::self_test();
}
