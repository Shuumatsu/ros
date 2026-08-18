//! How a physical frame is reached from kernel context.
//!
//! A page-table walk must *dereference* the frames it descends through, but a
//! physical address is not in general a usable pointer: a higher-half kernel
//! reaches physical memory through a mapping at some other virtual address.
//! Naming that policy here is what lets [`crate::mapper`] walk a tree without
//! assuming one — and what keeps this crate free of any addressing model of
//! its own.

use crate::addr::PhysicalAddr;

/// Turns a physical address into a pointer the caller can dereference.
///
/// # Safety
///
/// An implementation must return a pointer valid for reads and writes of `T`
/// at `pa`, for as long as that frame is reachable from a live page table. The
/// mapper dereferences the result directly, so a wrong answer is undefined
/// behaviour, not merely a wrong mapping.
pub unsafe trait PhysAccess {
    /// Reach the frame at `pa` as a `*mut T`.
    fn ptr<T>(&self, pa: PhysicalAddr) -> *mut T;
}

/// Physical addresses are usable as pointers directly.
///
/// Correct while the kernel runs identity-mapped — during early boot before a
/// high-half mapping is live, and in host tests, where a "physical address" is
/// really a host pointer.
#[derive(Clone, Copy, Debug, Default)]
pub struct Identity;

// SAFETY: the caller constructing a `Mapper` with this strategy promises the
// identity mapping is live, which makes a physical address a valid pointer.
unsafe impl PhysAccess for Identity {
    #[inline]
    fn ptr<T>(&self, pa: PhysicalAddr) -> *mut T { pa.bits() as *mut T }
}

/// Physical memory is mapped at a fixed virtual offset: `VA = PA + offset`.
///
/// The higher-half kernel's view of RAM.
#[derive(Clone, Copy, Debug)]
pub struct LinearOffset(pub usize);

// SAFETY: the caller promises `offset` describes a live mapping covering every
// frame the walk can reach.
unsafe impl PhysAccess for LinearOffset {
    #[inline]
    fn ptr<T>(&self, pa: PhysicalAddr) -> *mut T { pa.bits().wrapping_add(self.0) as *mut T }
}
