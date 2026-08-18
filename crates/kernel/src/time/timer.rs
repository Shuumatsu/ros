//! The periodic tick: how often one fires, and what one costs.
//!
//! A hart arms its own timer and takes its own interrupts, so everything here is per hart and
//! nothing is coordinated: [`start`] runs once on each, and [`tick`] runs on whichever hart
//! the interrupt reached. What one tick *means* stays open until there is a scheduler to
//! preempt — for now it is the kernel's proof that traps work.
//!
//! Timekeeping is not this module's: the counter is the clock, read directly through
//! [`arch::time_counter`]. Each tick re-arms from the counter's current value, so a tick
//! handled late shifts the ones after it, and nothing derives a time from how many there
//! have been.

use core::sync::atomic::{AtomicU64, Ordering};

use spin::Once;

use crate::arch::{self, interrupts, timer};
use crate::cpu::{self, MAX_CPUS};

/// Ticks per second, on every hart.
pub const HZ: u64 = 100;

/// One-second marks each hart reports before going quiet.
///
/// The tick is a bring-up proof, and a proof does not have to repeat: four harts printing
/// once a second forever would bury every log that follows.
const REPORTED_SECONDS: u64 = 3;

/// Counter ticks between interrupts. One timebase serves the whole machine, so the first
/// hart to work it out decides it for all of them.
static INTERVAL: Once<u64> = Once::new();

/// Ticks taken, per cpu slot. Atomic because it is a shared static, not because it is
/// contended: a hart touches only its own slot, so a load and a store are enough where a
/// counter shared between harts would need a read-modify-write.
static TICKS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];

/// Arm this hart's timer and let the interrupt through.
///
/// Per hart, after [`crate::trap::init`]: the source is unmasked here, so a vector and a
/// dispatcher have to exist first. Arming comes before unmasking, since a source enabled
/// with no deadline behind it is one whose first interrupt is the platform's guess.
///
/// A machine that reports no timebase, or firmware that refuses the call, leaves the hart
/// without a tick. Reported and survivable, since nothing schedules yet.
pub fn start() {
    let Some(hz) = super::hz() else {
        println!("[timer] no timebase reported: this hart runs without a tick");
        return;
    };

    // `max(1)` because the interval is firmware input divided by ours: a timebase slower than
    // HZ would otherwise ask for a deadline that has already passed, forever.
    let interval = *INTERVAL.call_once(|| (hz / HZ).max(1));

    if let Err(error) = timer::set_next_event(arch::time_counter() + interval) {
        println!(
            "[timer] firmware refused a timer deadline ({error:?}): this hart runs without a tick"
        );
        return;
    }

    // SAFETY: `crate::trap` dispatches the timer to `tick` below, and the deadline above is
    // set, so the first interrupt is one this kernel asked for and knows what to do with.
    unsafe { interrupts::enable_timer() };

    println!("[timer] {HZ} Hz on this hart ({interval} counter ticks per interval)");
}

/// One timer interrupt.
///
/// Re-arming first, because that is also the acknowledgement: `sip.STIP` is read-only to a
/// supervisor, so until the next deadline is set the interrupt is still pending and returning
/// would take it again.
pub(crate) fn tick() {
    let interval = *INTERVAL.get().expect("a timer interrupt before timer::start armed one");
    timer::set_next_event(arch::time_counter() + interval)
        .expect("firmware took the first timer deadline on this hart and then refused one");

    let slot = &TICKS[cpu::current().index()];
    let ticks = slot.load(Ordering::Relaxed) + 1;
    slot.store(ticks, Ordering::Relaxed);

    let seconds = ticks / HZ;
    if ticks.is_multiple_of(HZ) && seconds <= REPORTED_SECONDS {
        println!("[timer] tick {ticks} ({seconds}s)");
    }
}
