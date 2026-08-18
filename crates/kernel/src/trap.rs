//! What the kernel does about a trap.
//!
//! The mechanism is [`crate::arch::trap`]'s: where a trap lands, what it saves, and what
//! `scause` meant. What each cause *costs* is here, so a new handler is a change to this file
//! and not to the ISA layer.
//!
//! One dispatch point, [`handle`], reached with this hart's interrupts already masked by
//! hardware — `sstatus.SIE` is cleared on entry and restored by `sret`. So no handler nests,
//! and a lock a handler takes cannot be taken again by the same handler on the same hart.

use crate::arch::interrupts;
use crate::arch::trap::{self, Cause, Fault, TrapFrame};

/// Take traps on this hart.
///
/// Per hart, from both Rust entries, once the kernel page table is live: the vector is a
/// kernel virtual address and `stvec` is a CSR.
///
/// **No interrupt source is enabled here.** This replaces the boot stage's park vector with a
/// dispatcher and lets interrupts through in principle; which sources exist is each source's
/// own module to say, and [`crate::start`] owns the order they are armed in.
pub fn init() {
    trap::install();

    // SAFETY: firmware does not promise `sie` is clear on a hart it starts, and the vector
    // installed above dispatches nothing but the timer. Masking first is what makes "no
    // source is enabled" a fact.
    unsafe { interrupts::mask_all_sources() };
    // SAFETY: the vector is installed, so an interrupt taken from here reaches `handle`.
    unsafe { interrupts::enable() };

    println!("[trap] stvec -> {:#x}, interrupts unmasked, no source enabled yet", trap::vector());
}

/// Every trap the hardware can deliver, and what this kernel does with it.
pub(crate) fn handle(cause: Cause, frame: &mut TrapFrame) {
    match cause {
        Cause::Timer => crate::time::timer::tick(),
        // Nothing enables either source, so arriving here means one was unmasked without a
        // handler — worth naming, because the alternative is an interrupt that fires forever
        // with nobody to acknowledge it.
        Cause::Software => panic!("supervisor software interrupt with no handler"),
        Cause::External => panic!("supervisor external interrupt with no handler"),
        Cause::Fault(fault) => fatal(&fault, frame),
    }
}

/// A trap the kernel cannot continue from: say what the CSRs and the register file say, then
/// die on this hart.
///
/// The lock-free console, because the fault may be one *inside* `_print` on a hart that
/// already holds the console lock, where the locked path would deadlock instead of reporting.
/// Output that interleaves with another hart's beats no output.
fn fatal(fault: &Fault, frame: &TrapFrame) -> ! {
    emergency_println!("[trap] the register file at the fault:\r\n{frame}");
    panic!("unhandled supervisor trap: {fault}");
}
