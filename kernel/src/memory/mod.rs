use core::alloc::{GlobalAlloc, Layout};

use lazy_static::lazy_static;
use log::debug;
use spin::Mutex;

mod allocator;
pub mod layout;
mod paging;

use allocator::Allocator;
use layout::{HEAP_END, HEAP_START};

lazy_static! {
    static ref ALLOCATOR: Mutex<Allocator> = unsafe {
        debug!("[allocator] initializing global heap allocator...");

        let allocator = Allocator::new(*HEAP_START, *HEAP_END);

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
        // println!("allocating memory for {:?}", layout);
        let r = ALLOCATOR.lock().alloc(layout);
        // println!("[OsAllocator] allocated memory for {:?}, at {:?}", layout, r);
        r
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        ALLOCATOR.lock().dealloc(ptr, layout);
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

pub unsafe fn memset(ptr: *mut u8, ch: u8, count: usize) {
    for _ in 0..count {
        *ptr = ch;
    }
}
