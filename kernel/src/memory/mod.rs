use core::alloc::{GlobalAlloc, Layout};
use core::mem::size_of_val;

use buddy_system_allocator::{LockedFrameAllocator, LockedHeap};
use lazy_static::lazy_static;
use log::{debug, info, trace};
use spin::Mutex;

pub mod addr;
pub mod layout;
mod paging;

use crate::config::{KERNEL_HEAP_SIZE, PAGE_SIZE};
use layout::{HEAP_END, HEAP_START};

lazy_static! {
    static ref FRAME_ALLOCATOR: LockedFrameAllocator = LockedFrameAllocator::new();
}

static mut KERNEL_HEAP: [u8; KERNEL_HEAP_SIZE] = [0; KERNEL_HEAP_SIZE];

#[global_allocator]
static HEAP_ALLOCATOR: LockedHeap<31> = LockedHeap::empty();

pub fn init() {
    unsafe {
        HEAP_ALLOCATOR
            .lock()
            .init(&KERNEL_HEAP as *const _ as _, size_of_val(&KERNEL_HEAP));
    }
    unsafe {
        FRAME_ALLOCATOR.lock().add_frame(
            extract_range!(*HEAP_START, 12, 56),
            extract_range!(*HEAP_END, 12, 56),
        );
        println!(
            "frames_cnt: {}",
            extract_range!(*HEAP_END, 12, 56) - extract_range!(*HEAP_START, 12, 56)
        );
    }
    unsafe {
        paging::init();
    }
}

#[alloc_error_handler]
pub fn alloc_error(l: Layout) -> ! {
    panic!(
        "Allocator failed to allocate {} bytes with {}-byte alignment.",
        l.size(),
        l.align()
    );
}
