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
    let end = layout::memory_end();

    unsafe {
        HEAP.lock().add_to_heap(start, end);
    }

    println!("[memory] heap initialized: {:#x} - {:#x} ({} MB)", start, end, (end - start) / 1024 / 1024);
}
