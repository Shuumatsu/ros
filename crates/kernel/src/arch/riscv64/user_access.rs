//! `sstatus.SUM`: whether a supervisor load or store may reach a page marked `USER`.
//!
//! Interrupts remain masked while `SUM` is set, and `SUM` is cleared before they are restored.

use riscv::register::sstatus;

use super::interrupts;

/// Runs `f` with supervisor access to user pages enabled on this hart.
pub fn with<R>(f: impl FnOnce() -> R) -> R {
    interrupts::without(|| {
        // SAFETY: widening what this hart's own loads may reach, for the length of one call.
        // Whether the addresses `f` touches are a process's is `f`'s to have established.
        unsafe { sstatus::set_sum() };
        let result = f();
        // SAFETY: clearing the bit this function set, on the same hart.
        unsafe { sstatus::clear_sum() };
        result
    })
}
