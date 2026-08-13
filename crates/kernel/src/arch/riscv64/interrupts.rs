//! Supervisor interrupt masking: the only place in the kernel that writes `sstatus.sie`.
//!
//! A closure rather than a token, so the restore cannot be skipped by an early return. A
//! `Drop` guard would read better but guarantee no more — `panic = "abort"` means nothing
//! unwinds, so it would not run on the panic path either.

use riscv::register::sstatus;

/// Run `f` with supervisor interrupts masked on this hart, then restore the previous
/// state.
///
/// Restores rather than unconditionally enables: callers nest, and the boot path runs
/// with interrupts off long before anything enables them. Unconditionally setting
/// `sie` on the way out would turn "leave it as you found it" into "turn it on", which
/// during memory bring-up would enable interrupts before there is a handler.
pub fn without<R>(f: impl FnOnce() -> R) -> R {
    let was_enabled = sstatus::read().sie();
    if was_enabled {
        // SAFETY: masking interrupts cannot violate memory safety; the only hazard is
        // leaving them masked, which the restore below prevents.
        unsafe { sstatus::clear_sie() };
    }

    let result = f();

    if was_enabled {
        // SAFETY: restoring the bit this function cleared, on the same hart.
        unsafe { sstatus::set_sie() };
    }
    result
}
