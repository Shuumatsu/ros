use alloc::vec::Vec;
use spin::Mutex;

use crate::config::PAGE_SIZE;
use crate::memory::addr::{PhysicalAddr, VirtualAddr};

#[derive(Debug, Default)]
pub struct FrameAllocator {
    frames: Vec<u64>,
}

impl FrameAllocator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn alloc(&mut self) -> Option<u64> {
        self.frames.pop()
    }

    pub fn dealloc(&mut self, ppn: u64) {
        self.frames.push(ppn)
    }
}
