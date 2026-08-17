//! Locks that are safe to take on a hart that also takes interrupts.
//!
//! A `spin::Mutex` protects against other harts, not against the holder: an interrupt
//! arriving mid-critical-section whose handler wants the same lock spins for a release
//! that cannot come until it returns. The frame allocator and the heap are both reached
//! from ordinary code *and* from trap handlers, so both are that shape.

use crate::arch::interrupts;

/// A spin lock that masks the holder's interrupts for as long as it is held.
pub struct IrqMutex<T> {
    inner: spin::Mutex<T>,
}

impl<T> IrqMutex<T> {
    /// Wrap `value`. `const`, so this can protect a `static`.
    pub const fn new(value: T) -> Self { Self { inner: spin::Mutex::new(value) } }

    /// Run `f` on the protected value, locked and with interrupts masked.
    ///
    /// A closure rather than a guard for [`interrupts::without`]'s reason — `panic =
    /// "abort"` means `Drop` is not a guarantee — and because it fixes the release order.
    /// Not reentrant: `f` must not reach the same `IrqMutex`.
    pub fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        interrupts::without(|| f(&mut self.inner.lock()))
    }
}
