//! Kernel trap policy.
//!
//! Hardware masks interrupts during dispatch, preventing nested handlers on the same hart.
//! The saved frame determines the interrupted privilege level.

use crate::arch::interrupts;
use crate::arch::trap::{self, Cause, Fault, TrapFrame};
use crate::process;

/// Installs trap state and unmasks interrupts without enabling any source.
pub fn init() {
    trap::install();
    trap::allow_user_counters();

    // SAFETY: no subsystem has enabled a source during hart initialization.
    unsafe { interrupts::mask_all_sources() };
    // SAFETY: the vector is installed, so an interrupt taken from here reaches `handle`.
    unsafe { interrupts::enable() };

    println!("[trap] stvec -> {:#x}, interrupts unmasked, no source enabled yet", trap::vector());
}

pub(crate) fn handle(cause: Cause, frame: &mut TrapFrame) {
    match cause {
        Cause::Timer => {
            if frame.interrupted_user() {
                process::record_user_tick();
            }
            crate::time::timer::tick()
        }
        Cause::Syscall => crate::syscall::dispatch(frame),
        Cause::Software => panic!("supervisor software interrupt with no handler"),
        Cause::External => panic!("supervisor external interrupt with no handler"),
        Cause::Fault if frame.interrupted_user() => faulted(frame),
        Cause::Fault => fatal(frame),
    }
}

fn faulted(frame: &TrapFrame) -> ! {
    println!("[trap] the running process faulted: {}\n{frame}", Fault::current());
    process::kill()
}

/// Reports a fatal trap without acquiring the potentially faulting console lock.
fn fatal(frame: &TrapFrame) -> ! {
    let fault = Fault::current();
    emergency_println!("[trap] the register file at the fault:\n{frame}");
    panic!("unhandled supervisor trap: {fault}");
}
