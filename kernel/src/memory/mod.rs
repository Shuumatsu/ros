use core::alloc::{GlobalAlloc, Layout};

use lazy_static::lazy_static;
use log::debug;
use spin::Mutex;

mod frame_allocator;
mod heap_allocator;
pub mod layout;
mod paging;

use crate::config::{KERNEL_HEAP_SIZE, PAGE_SIZE};
use frame_allocator::FrameAllocator;
use heap_allocator::Allocator;
use layout::{HEAP_END, HEAP_START};

const KERNEL_HEAP: [u8; KERNEL_HEAP_SIZE] = [0; KERNEL_HEAP_SIZE];

lazy_static! {
    static ref HEAP_ALLOCTOR: Mutex<Allocator> = unsafe {
        debug!("[allocator] initializing global heap allocator...");

        let allocator = Allocator::new(
            &KERNEL_HEAP as *const _ as _,
            &KERNEL_HEAP.last().unwrap() as *const _ as _,
        );

        println!(
            "[allocator] global heap allocator created at {:#x}",
            &allocator as *const _ as usize
        );

        Mutex::new(allocator)
    };
}

struct OsAllocator;

unsafe impl GlobalAlloc for OsAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        HEAP_ALLOCTOR.lock().alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        HEAP_ALLOCTOR.lock().dealloc(ptr, layout);
    }
}

#[global_allocator]
static GA: OsAllocator = OsAllocator;

#[alloc_error_handler]
pub fn alloc_error(l: Layout) -> ! {
    panic!(
        "Allocator failed to allocate {} bytes with {}-byte alignment.",
        l.size(),
        l.align()
    );
}

lazy_static! {
    pub static ref FRAME_ALLOCATOR: Mutex<FrameAllocator> = Mutex::new({
        let mut allocator = FrameAllocator::new();
        for phys_no in (*HEAP_START..*HEAP_END).step_by(PAGE_SIZE) {
            allocator.dealloc(phys_no);
        }
        allocator
    });
}
