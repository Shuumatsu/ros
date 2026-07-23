use crate::println;
use buddy_system_allocator::LockedHeap;
use core::alloc::Layout;

pub mod layout;
pub mod paging;

/// ORDER determines max allocation size: 2^(ORDER-1) bytes
/// ORDER=32 supports up to 2GB allocations
const ORDER: usize = 32;

#[global_allocator]
static HEAP: LockedHeap<ORDER> = LockedHeap::empty();

#[alloc_error_handler]
fn alloc_error(layout: Layout) -> ! {
    panic!(
        "Heap allocation failed: {} bytes with {}-byte alignment",
        layout.size(),
        layout.align()
    );
}

/// Initialize the memory subsystem
pub fn init() {
    let start = layout::heap_start();
    // The RAM top is discovered from the device tree by `device_tree::init`,
    // which panics if the tree is unusable — so by the time we get here it is
    // authoritative. No compile-time fallback: a wrong size corrupts the heap.
    let end = crate::device_tree::ram_end()
        .expect("device tree RAM region not discovered; call device_tree::init before memory::init");

    unsafe {
        HEAP.lock().add_to_heap(start, end);
    }

    println!(
        "[memory] heap: {:#x} - {:#x} ({} MB)",
        start,
        end,
        (end - start) / 1024 / 1024
    );
}
