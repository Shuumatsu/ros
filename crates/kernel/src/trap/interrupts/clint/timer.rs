//! Supervisor timer via the SBI TIME (legacy `set_timer`) extension.
//!
//! We are an S-mode payload; the M-mode CLINT belongs to the SBI firmware. So we
//! ask it for the next timer interrupt with `sbi::set_timer` and re-arm on each
//! tick. `rdtime` reads the `time` CSR, which is readable in S-mode because
//! OpenSBI sets `[m|s]counteren.TM`.

use core::sync::atomic::{AtomicU64, Ordering};

use crate::arch::riscv64::sbi;
use crate::trap::TrapFrame;

/// Cycles between ticks. QEMU virt's timebase is 10 MHz, so this is ~1 s.
pub const INTERVAL: u64 = 10_000_000;

/// Current time (the `time` CSR / `mtime`), readable in S-mode.
#[inline]
fn now() -> u64 {
    let t: u64;
    unsafe { core::arch::asm!("rdtime {}", out(reg) t, options(nomem, nostack)) };
    t
}

/// Arm the first timer interrupt.
pub fn init() {
    sbi::set_timer(now() + INTERVAL);
}

static TICKS: AtomicU64 = AtomicU64::new(0);

/// S-mode timer interrupt handler: re-arm for the next tick and account for this
/// one. The SBI `set_timer` call also clears the pending timer interrupt.
pub fn handler(_tf: &mut TrapFrame) {
    sbi::set_timer(now() + INTERVAL);
    let n = TICKS.fetch_add(1, Ordering::Relaxed) + 1;
    println!("[timer] tick {}", n);
}
