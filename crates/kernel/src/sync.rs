//! Spin locks that mask local interrupts while held.

use crate::arch::interrupts;

/// A spin lock that masks the holder's interrupts for as long as it is held.
pub struct IrqMutex<T> {
    inner: spin::Mutex<T>,
}

impl<T> IrqMutex<T> {
    pub const fn new(value: T) -> Self { Self { inner: spin::Mutex::new(value) } }

    /// Run `f` on the protected value, locked and with interrupts masked.
    ///
    /// `f` must not re-enter this `IrqMutex`.
    pub fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        interrupts::without(|| f(&mut self.inner.lock()))
    }
}
