//! Time: a free-running counter, the frequency that gives it units, and the periodic tick.
//!
//! Reading the counter is [`arch::time_counter`]'s; what a tick is worth is this module's.
//! The platform is allowed not to say, so every answer derived from the frequency is an
//! `Option` — a caller asking for "two seconds" gets an answer or an honest `None` rather
//! than a raw counter and a conversion to get wrong.
//!
//! [`deadline`] and [`spin_until`] need nothing but the counter, so they work before there is
//! a trap subsystem: [`crate::cpu`] waits on secondary harts with them during bring-up.
//! [`timer`] is the other half, and needs a trap vector and a handler.

pub mod timer;

use crate::arch;
use crate::device_tree;

/// The counter's frequency in Hz, or `None` when the machine reports no timebase.
///
/// The kernel's one reader of it: [`deadline`] turns seconds into counter ticks with it and
/// [`timer`] turns it into an interval, and a second lookup would be a second chance to
/// disagree about the rate the counter runs at.
pub fn hz() -> Option<u64> { device_tree::timebase_hz().map(|hz| hz as u64) }

/// A point on the counter, as produced by [`deadline`].
#[derive(Clone, Copy, Debug)]
pub struct Deadline(u64);

/// A deadline `secs` seconds from now, or `None` when the machine reports no timebase and
/// so no wait can be bounded at all.
///
/// Saturating, because the frequency is firmware input: an absurd one yields a deadline
/// that never passes rather than one already in the past.
pub fn deadline(secs: u64) -> Option<Deadline> {
    let hz = hz()?;
    Some(Deadline(arch::time_counter().saturating_add(secs.saturating_mul(hz))))
}

/// Spin until `ready` holds or `deadline` passes, whichever comes first. `true` means
/// `ready`.
///
/// `ready` is polled once more after the deadline, so a condition that came true in the
/// last instant of the wait is not reported as a timeout.
#[must_use]
pub fn spin_until(deadline: Deadline, mut ready: impl FnMut() -> bool) -> bool {
    while arch::time_counter() < deadline.0 {
        if ready() {
            return true;
        }
        core::hint::spin_loop();
    }
    ready()
}
