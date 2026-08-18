//! Supervisor interrupt masking: the only place in the kernel that writes `sstatus.SIE` or
//! `sie`.
//!
//! Two levels, and they are not interchangeable. `sstatus.SIE` is this hart's one switch —
//! whether it takes interrupts at all — and [`without`] borrows it for a critical section.
//! `sie` is the per-source mask, and a source stays off until its own subsystem has a
//! handler ready, which is what [`enable_timer`] is for. One function per source, appearing
//! with the handler that justifies it, so the register never carries a bit nobody dispatches.
//!
//! Owning both here rather than one per source is what keeps `sie` a single answer: the bits
//! live in one CSR, so scattering the writes would scatter "which sources this kernel takes".

use riscv::interrupt::supervisor::Interrupt;
use riscv::register::{sie, sstatus};

/// Run `f` with supervisor interrupts masked on this hart, then restore the previous
/// state.
///
/// Restores rather than unconditionally enables: callers nest, and the boot path runs
/// with interrupts off long before anything enables them. Unconditionally setting
/// `SIE` on the way out would turn "leave it as you found it" into "turn it on", which
/// during memory bring-up would enable interrupts before there is a handler.
///
/// A closure rather than a token, so the restore cannot be skipped by an early return. A
/// `Drop` guard would read better but guarantee no more — `panic = "abort"` means nothing
/// unwinds, so it would not run on the panic path either.
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

/// Take interrupts on this hart from now on.
///
/// # Safety
///
/// A trap vector must be installed first, or the first interrupt goes wherever `stvec`
/// happens to point.
pub unsafe fn enable() {
    // SAFETY: forwarded from this function's contract.
    unsafe { sstatus::set_sie() };
}

/// Stop taking interrupts on this hart. The fatal path's, so that a parked hart stays
/// parked; ordinary code borrows the bit through [`without`] instead.
pub unsafe fn disable() {
    // SAFETY: masking interrupts on the calling hart is sound on its own; what it can
    // break is liveness, which is the caller's to judge.
    unsafe { sstatus::clear_sie() };
}

/// Mask every interrupt source on this hart.
///
/// Firmware does not promise `sie` is clear on a hart it starts, so this is what makes
/// "no source is enabled" true rather than assumed. Writes the whole register: the point is
/// the sources this kernel has never heard of, not the three it names.
///
/// # Safety
///
/// Silences every source, including ones another subsystem may be relying on. Call while
/// bringing a hart up, before anything has enabled one.
pub unsafe fn mask_all_sources() {
    // SAFETY: forwarded from this function's contract.
    unsafe { sie::write(sie::Sie::from_bits(0)) };
}

/// Let the supervisor timer through on this hart.
///
/// # Safety
///
/// A deadline must be armed and `crate::trap` must dispatch the timer to a handler: an
/// enabled source with neither fires immediately and cannot be acknowledged.
pub unsafe fn enable_timer() {
    let mut mask = sie::read();
    mask.enable(Interrupt::SupervisorTimer);
    // SAFETY: forwarded from this function's contract. Read-modify-write needs no lock —
    // `sie` is per-hart, and a handler that could change it underneath us would be one this
    // hart cannot take, since the source it would come from is still masked.
    unsafe { sie::write(mask) };
}
