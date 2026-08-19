//! What the kernel does about a trap.
//!
//! The mechanism is [`crate::arch::trap`]'s: where a trap lands, what it saves, and what
//! `scause` meant. What each cause *costs* is here, so a new handler is a change to this file
//! and not to the ISA layer.
//!
//! One dispatch point, [`handle`], reached with this hart's interrupts already masked by
//! hardware — `sstatus.SIE` is cleared on entry and restored by `sret`. So no handler nests,
//! and a lock a handler takes cannot be taken again by the same handler on the same hart.
//!
//! What privilege level a trap came from is a fact about the interrupted context, so it is read off
//! the frame rather than tracked here. It changes what two causes cost and nothing else: a fault
//! ends the process instead of the hart, and a tick is also evidence that a process was preempted.

use crate::arch::interrupts;
use crate::arch::trap::{self, Cause, Fault, TrapFrame};
use crate::process;

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
    // With the vector, because both are what a hart owes before user mode can run on it, and both
    // are CSRs.
    trap::allow_user_counters();

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
        Cause::Timer => {
            if frame.from_user() {
                process::record_user_tick();
            }
            crate::time::timer::tick()
        }
        Cause::Syscall => crate::syscall::dispatch(frame),
        // Nothing enables either source, so arriving here means one was unmasked without a
        // handler — worth naming, because the alternative is an interrupt that fires forever
        // with nobody to acknowledge it.
        Cause::Software => panic!("supervisor software interrupt with no handler"),
        Cause::External => panic!("supervisor external interrupt with no handler"),
        // A fault the running program caused costs the program. One the kernel caused costs the
        // hart, because there is nothing smaller left to charge it to.
        Cause::Fault(fault) if frame.from_user() => faulted(&fault, frame),
        Cause::Fault(fault) => fatal(&fault, frame),
    }
}

/// A fault in user mode: the process cannot go on, and the kernel can.
///
/// The locked console: user mode holds no kernel lock, so the deadlock [`fatal`] avoids is not in
/// the way, and a program failing is a routine event.
fn faulted(fault: &Fault, frame: &TrapFrame) -> ! {
    println!("[trap] the running process faulted: {fault}\n{frame}");
    process::kill()
}

/// A trap the kernel cannot continue from: say what the CSRs and the register file say, then
/// die on this hart.
///
/// The lock-free console, because the fault may be one *inside* `_print` on a hart that
/// already holds the console lock, where the locked path would deadlock instead of reporting.
/// Output that interleaves with another hart's beats no output.
fn fatal(fault: &Fault, frame: &TrapFrame) -> ! {
    emergency_println!("[trap] the register file at the fault:\n{frame}");
    panic!("unhandled supervisor trap: {fault}");
}
