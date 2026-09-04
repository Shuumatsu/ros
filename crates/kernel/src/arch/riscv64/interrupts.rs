//! Supervisor interrupt masking.

use riscv::interrupt::supervisor::Interrupt;
use riscv::register::{sie, sstatus};

/// `sstatus.SIE`, encoded as the immediate the masking instructions below take.
const SIE: usize = 1 << 1;

/// Runs `f` with supervisor interrupts masked, then restores the previous state.
pub fn without<R>(f: impl FnOnce() -> R) -> R {
    let previous: usize;
    // SAFETY: reads `sstatus` and clears `SIE` in one instruction; the bit is restored below.
    unsafe {
        core::arch::asm!(
            "csrrci {previous}, sstatus, {sie}",
            previous = out(reg) previous,
            sie = const SIE,
            options(nostack)
        )
    };

    let result = f();

    // SAFETY: sets the bit this function cleared, on the same hart. A zero mask sets nothing,
    // so interrupts that were already masked stay masked.
    unsafe {
        core::arch::asm!("csrs sstatus, {mask}", mask = in(reg) previous & SIE, options(nostack))
    };
    result
}

/// # Safety
///
/// A valid trap vector must already be installed on this hart.
pub unsafe fn enable() {
    // SAFETY: forwarded from this function's contract.
    unsafe { sstatus::set_sie() };
}

/// # Safety
///
/// The caller must ensure permanently masking interrupts cannot block required progress.
pub unsafe fn disable() {
    // SAFETY: forwarded from this function's contract.
    unsafe { sstatus::clear_sie() };
}

/// # Safety
///
/// No subsystem may rely on an enabled interrupt source.
pub unsafe fn mask_all_sources() {
    // SAFETY: forwarded from this function's contract.
    unsafe { sie::write(sie::Sie::from_bits(0)) };
}

/// # Safety
///
/// A timer deadline and handler must already be installed.
pub unsafe fn enable_timer() {
    let mut mask = sie::read();
    mask.enable(Interrupt::SupervisorTimer);
    // SAFETY: forwarded from this function's contract; `sie` is per-hart.
    unsafe { sie::write(mask) };
}
