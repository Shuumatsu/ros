//! User-range validation and supervisor access.
//!
//! Validation checks only the canonical user half; an unmapped in-range address faults.

use mmu::{MemoryAddr, Scheme, VirtualAddr};

use super::KernelScheme;
use crate::arch::user_access;

/// Exclusive end of the canonical user half.
pub const END: VirtualAddr = VirtualAddr::new(KernelScheme::HALF_SPAN);

/// Return whether `[base, base + len)` lies in the user half without wrapping.
pub fn contains(base: VirtualAddr, len: usize) -> bool {
    base.checked_add(len).is_some_and(|end| end <= END)
}

/// Run `f` on an in-range user slice, or return `None` if the range is invalid.
///
/// Unmapped addresses fault. The slice cannot escape the callback or supervisor-access window.
pub fn read<R>(base: VirtualAddr, len: usize, f: impl FnOnce(&[u8]) -> R) -> Option<R> {
    if !contains(base, len) {
        return None;
    }
    if len == 0 {
        return Some(f(&[]));
    }

    Some(user_access::with(|| {
        // SAFETY: the range cannot alias kernel memory, supervisor access is enabled, and the
        // slice is confined to `f`; unmapped pages follow the kernel's fault path.
        let bytes = unsafe { core::slice::from_raw_parts(base.as_ptr::<u8>(), len) };
        f(bytes)
    }))
}
