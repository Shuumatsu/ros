//! The hart's timebase: a free-running counter, and the deadline that interrupts it.

use super::sbi;

/// Reads the free-running counter.
pub fn now() -> u64 { riscv::register::time::read64() }

/// Arms this hart's timer `ticks` counter units from now, clearing any pending interrupt.
pub fn arm_after(ticks: u64) -> Result<(), sbi::Error> { sbi::set_timer(now() + ticks) }
