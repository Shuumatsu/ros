//! Policies for dereferencing physical frames.

use crate::addr::PhysicalAddr;

/// Converts a physical address into a dereferenceable pointer.
///
/// # Safety
///
/// The returned pointer must be valid and properly aligned for reads and writes
/// of `T` while the frame is reachable from the page-table tree.
pub unsafe trait PhysAccess {
    fn ptr<T>(&self, pa: PhysicalAddr) -> *mut T;
}

/// Physical memory is mapped at a fixed virtual offset: `VA = PA + offset`.
///
/// `LinearOffset(0)` is the identity mapping.
#[derive(Clone, Copy, Debug)]
pub struct LinearOffset(pub usize);

// SAFETY: callers must provide a live offset mapping for every reachable frame.
unsafe impl PhysAccess for LinearOffset {
    #[inline]
    fn ptr<T>(&self, pa: PhysicalAddr) -> *mut T { pa.bits().wrapping_add(self.0) as *mut T }
}
