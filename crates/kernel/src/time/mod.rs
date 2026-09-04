//! Counter-based timekeeping and periodic timer support.

pub mod timer;

use crate::arch::timebase;
use crate::device_tree;

pub fn hz() -> Option<u64> { device_tree::timebase_hz() }

#[derive(Clone, Copy, Debug)]
pub struct Deadline(u64);

/// Returns a saturating deadline, or `None` when no timebase is reported.
pub fn deadline(secs: u64) -> Option<Deadline> {
    let hz = hz()?;
    Some(Deadline(timebase::now().saturating_add(secs.saturating_mul(hz))))
}

/// Spins until `ready` succeeds or the deadline passes, polling once at the deadline.
#[must_use]
pub fn spin_until(deadline: Deadline, mut ready: impl FnMut() -> bool) -> bool {
    while timebase::now() < deadline.0 {
        if ready() {
            return true;
        }
        core::hint::spin_loop();
    }
    ready()
}
