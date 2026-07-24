use riscv::interrupt::supervisor::Exception;

use crate::trap::TrapFrame;

mod breakpoint;
mod ecall;

/// Exception setup. Delegation (`medeleg`) is done by the SBI firmware in
/// M-mode, so in S-mode there is nothing to configure here — the trap vector is
/// installed by `trap::init`.
pub unsafe fn init() {}

pub fn handler(e: Exception, tf: &mut TrapFrame) {
    match e {
        Exception::Breakpoint => breakpoint::handler(e, tf),
        Exception::UserEnvCall => ecall::handler(tf),
        _ => panic!("unexpected exception: {:?}", e),
    }
}
