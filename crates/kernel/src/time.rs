//! Bounded waits: the `time` counter, and the frequency that gives it units.
//!
//! Both halves in one module because neither is usable alone. `rdtime` counts at a rate
//! only the platform knows, and the platform is allowed not to say, so a caller asking for
//! "two seconds" gets an answer or an honest `None` rather than a raw counter and a
//! conversion to get wrong.
//!
//! A CSR read and a spin, nothing more: no timer interrupt, so this works before there is a
//! trap subsystem to take one.

use crate::device_tree;

/// A point on the `time` counter, as produced by [`deadline`].
#[derive(Clone, Copy, Debug)]
pub struct Deadline(u64);

/// The `time` CSR: a free-running counter, readable in S-mode. The only read of it.
fn ticks() -> u64 {
    let t: u64;
    // SAFETY: `rdtime` reads a counter and has no side effects.
    unsafe { core::arch::asm!("rdtime {}", out(reg) t, options(nomem, nostack)) };
    t
}

/// A deadline `secs` seconds from now, or `None` when the machine reports no timebase and
/// so no wait can be bounded at all.
///
/// Saturating, because the frequency is firmware input: an absurd one yields a deadline
/// that never passes rather than one already in the past.
pub fn deadline(secs: u64) -> Option<Deadline> {
    let hz = device_tree::timebase_hz()? as u64;
    Some(Deadline(ticks().saturating_add(secs.saturating_mul(hz))))
}

/// Spin until `ready` holds or `deadline` passes, whichever comes first. `true` means
/// `ready`.
///
/// `ready` is polled once more after the deadline, so a condition that came true in the
/// last instant of the wait is not reported as a timeout.
#[must_use]
pub fn spin_until(deadline: Deadline, mut ready: impl FnMut() -> bool) -> bool {
    while ticks() < deadline.0 {
        if ready() {
            return true;
        }
        core::hint::spin_loop();
    }
    ready()
}
