//! Per-hart periodic timer interrupts.
//!
//! Each interrupt schedules the next deadline from the current counter, so delays shift later ticks.

use spin::Once;

use crate::arch::{self, interrupts, timer};
use crate::cpu;

pub const HZ: u64 = 100;

const REPORTED_SECONDS: u64 = 3;

/// Shared interval derived from the machine-wide timebase.
static INTERVAL: Once<u64> = Once::new();

/// Arms and unmasks this hart's timer after trap initialization.
///
/// Missing timebase data or firmware rejection leaves the hart without timer interrupts.
pub fn start() {
    let Some(hz) = super::hz() else {
        println!("[timer] no timebase reported: this hart runs without a tick");
        return;
    };

    // Keep every interval in the future when the timebase is slower than `HZ`.
    let interval = *INTERVAL.call_once(|| (hz / HZ).max(1));

    if let Err(error) = timer::set_next_event(arch::time_counter() + interval) {
        println!(
            "[timer] firmware refused a timer deadline ({error:?}): this hart runs without a tick"
        );
        return;
    }

    // SAFETY: the timer handler is installed and the first deadline is armed.
    unsafe { interrupts::enable_timer() };

    println!("[timer] {HZ} Hz on this hart ({interval} counter ticks per interval)");
}

/// Handles one timer interrupt, rearming first to acknowledge `sip.STIP`.
pub(crate) fn tick() {
    let interval = *INTERVAL.get().expect("a timer interrupt before timer::start armed one");
    timer::set_next_event(arch::time_counter() + interval)
        .expect("firmware took the first timer deadline on this hart and then refused one");

    let ticks = cpu::current().record_tick();

    let seconds = ticks / HZ;
    if ticks.is_multiple_of(HZ) && seconds <= REPORTED_SECONDS {
        println!("[timer] tick {ticks} ({seconds}s)");
    }
}
