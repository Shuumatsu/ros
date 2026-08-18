//! When this hart's next timer interrupt lands.
//!
//! One function, because that is the whole of the platform's timer: a deadline on the same
//! free-running counter [`super::time_counter`] reads, and an interrupt when the counter
//! reaches it. How often to set one and what to do with it is [`crate::time::timer`]'s.
//!
//! The compare register is not ours. `mtimecmp` belongs to the M-mode CLINT, which on this
//! platform is the SBI firmware's, so a supervisor arms its timer by asking for one. That
//! also means the arming *is* the acknowledgement: `sip.STIP` is read-only to S-mode, and
//! setting the next deadline is what stops the current interrupt from firing again.
//!
//! Sstc would change only the body below. That extension adds `stimecmp`, writable in S-mode
//! once M-mode sets `menvcfg.STCE`, which turns the call into one CSR write and saves a trap
//! into firmware per tick. Taking it needs a probe — `sstc` in the device tree's
//! `riscv,isa-extensions` — and this seam is where the answer would be spent.

use super::sbi;

/// Ask for a timer interrupt when the counter reaches `deadline`.
///
/// `Err` means firmware refused, which on this path means the hart has no timer: there is no
/// second mechanism to fall back to, so the caller reports it rather than retrying.
pub fn set_next_event(deadline: u64) -> Result<(), sbi::Error> { sbi::set_timer(deadline) }
