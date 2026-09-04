//! Supervisor timer programming through SBI.

use super::sbi;

/// Arms the timer for `deadline`, acknowledging any pending timer interrupt.
pub fn set_next_event(deadline: u64) -> Result<(), sbi::Error> { sbi::set_timer(deadline) }
