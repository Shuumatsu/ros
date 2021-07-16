use alloc::vec::Vec;
use spin::Mutex;

use crate::config::PAGE_SIZE;
use crate::memory::layout::{HEAP_END, HEAP_START};

#[derive(Debug, Default)]
pub struct FrameAllocator {
    frames: Vec<usize>,
}

impl FrameAllocator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn alloc(&mut self) -> Option<usize> {
        self.frames.pop()
    }

    pub fn dealloc(&mut self, phys_no: usize) {
        self.frames.push(phys_no)
    }
}
