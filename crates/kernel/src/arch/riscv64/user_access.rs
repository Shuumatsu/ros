//! `sstatus.SUM`: whether a supervisor load or store may reach a page marked `USER`.
//!
//! The only place in the kernel that writes the bit, and it writes it in pairs: set on the way
//! into [`with`], clear on the way out. So every kernel access to a process's memory is one a
//! caller asked for by name, and a kernel pointer that has wandered into the low half faults
//! instead of quietly reading whatever the running process keeps there.
//!
//! Interrupts are masked across the window, which is what confines the bit to the closure: no
//! handler on this hart can run while it is set, and no other hart shares it.

use riscv::register::sstatus;

use super::interrupts;

/// Run `f` with this hart able to read and write the running process's pages.
///
/// A closure rather than a token for [`interrupts::without`]'s reason: `panic = "abort"` means a
/// `Drop` guard would not run on the path that matters, and a closure cannot be escaped by an
/// early return.
///
/// The bit is cleared on the way out rather than restored, because nothing else sets it: leaving
/// the window is leaving it off.
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
