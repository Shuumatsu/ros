//! Storage provider for intermediate page tables.

use crate::addr::PhysicalAddr;

/// Supplies frames for intermediate page tables.
///
/// # Safety
///
/// [`alloc_zeroed`](Self::alloc_zeroed) must return a page-aligned, fully
/// zeroed frame exclusively owned by the recipient until `free`.
pub unsafe trait FrameSource {
    fn alloc_zeroed(&mut self) -> Option<PhysicalAddr>;

    /// # Safety
    ///
    /// `frame` must have come from this source and must no longer be reachable
    /// from any live page table.
    unsafe fn free(&mut self, frame: PhysicalAddr);
}
