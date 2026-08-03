use riscv::interrupt::supervisor::Exception;

use crate::trap::TrapFrame;

mod breakpoint;
mod ecall;

/// Exception setup. Delegation (`medeleg`) is done by the SBI firmware in
/// M-mode, so in S-mode there is nothing to configure here — the trap vector is
/// installed by `trap::init`.
pub unsafe fn init() {}

/// Dispatch an exception. `epc` is the faulting PC, read once by [`crate::trap`].
pub fn handler(e: Exception, tf: &mut TrapFrame, epc: usize) {
    match e {
        // Deliberate traps: the program asked for these, so they are not errors and
        // nothing is reported.
        Exception::Breakpoint => breakpoint::handler(e, tf),
        Exception::UserEnvCall => ecall::handler(tf),
        // A fault. `epc` goes in the panic rather than a separate line above: the
        // panic handler already uses the emergency writer, so this is one message on
        // the fatal path instead of two on every path.
        _ => panic!("unexpected exception: {e:?} at sepc {epc:#x}"),
    }
}
