//! The half of the address space a process owns, and how the kernel reads out of it.
//!
//! A user address is a claim rather than a fact: it arrives in a register the process filled. What
//! this module owns is turning one into something the kernel may dereference — the check that says
//! it is a user address at all, and the window that lets a supervisor load reach a page marked
//! `USER`.
//!
//! The range check is the extent of the validation. An address inside the half that no leaf maps
//! faults, and the kernel reports that as a fault rather than as a bad argument.

use mmu::{MemoryAddr, Scheme, VirtualAddr};

use super::KernelScheme;
use crate::arch::user_access;

/// One past the highest address a process can name.
///
/// The scheme divides the address space into two canonical halves and gives the low one to the
/// process, so this is where its half stops. A pointer at or past it is one the kernel would be
/// dereferencing into itself.
pub const END: VirtualAddr = VirtualAddr::new(KernelScheme::HALF_SPAN);

/// Whether `[base, base + len)` is a range a process could own.
///
/// Rejects a length that would carry the range past the end of the address space as well as one
/// that reaches the kernel's half, since a wrap would otherwise make a short range out of a long
/// one.
pub fn contains(base: VirtualAddr, len: usize) -> bool {
    base.checked_add(len).is_some_and(|end| end <= END)
}

/// Read `len` bytes at `base` out of the running process, or `None` if that is not its memory.
///
/// The bytes reach `f` as a slice and go no further: the window closes when `f` returns, so
/// nothing the kernel keeps afterwards points into a process's pages.
pub fn read<R>(base: VirtualAddr, len: usize, f: impl FnOnce(&[u8]) -> R) -> Option<R> {
    if !contains(base, len) {
        return None;
    }
    // Nothing to reach into, so no window opens and no pointer is formed.
    if len == 0 {
        return Some(f(&[]));
    }

    Some(user_access::with(|| {
        // SAFETY: `len` bytes inside the low half, which the address space this hart is running
        // gives to the process alone, and the window above is what makes them readable from here.
        // The slice lives no longer than the call.
        let bytes = unsafe { core::slice::from_raw_parts(base.as_ptr::<u8>(), len) };
        f(bytes)
    }))
}
