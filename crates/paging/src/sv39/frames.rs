//! Where the frames holding intermediate page tables come from.
//!
//! This crate never allocates. A caller that maps anything below the root level
//! needs somewhere for the intermediate tables to live, and supplies it here.
//! In a kernel that is the physical frame allocator — page tables are exactly
//! page-sized, so they belong to the frame allocator and not to the heap. In
//! tests it can be a simple arena.

use super::addr::PhysicalAddr;

/// Supplies and reclaims the frames that intermediate page tables live in.
///
/// # Safety
///
/// [`alloc_zeroed`](Self::alloc_zeroed) must return a page-aligned frame that
/// is fully zeroed and exclusively owned by the caller until it is freed. A
/// frame that is already in use, unaligned, or non-zero corrupts the page table
/// that gets built in it — a zeroed frame is what makes a fresh table read as
/// "all entries invalid".
pub unsafe trait FrameSource {
    /// Obtain a zeroed, page-aligned frame for a new page table.
    fn alloc_zeroed(&mut self) -> Option<PhysicalAddr>;

    /// Return a frame previously handed out by [`alloc_zeroed`](Self::alloc_zeroed).
    ///
    /// # Safety
    ///
    /// `frame` must have come from this source and must no longer be reachable
    /// from any live page table.
    unsafe fn free(&mut self, frame: PhysicalAddr);
}
