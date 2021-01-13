use riscv::register::{medeleg, scause::Exception};

use crate::trap::TrapFrame;

mod breakpoint;

pub unsafe fn init() {
    kprintln!("delegate all exceptions to supervisor mode");
    asm!("li t0, 0xffff");
    asm!("csrw medeleg, t0");
}

pub fn handler(e: Exception, tf: &mut TrapFrame) {
    match e {
        Exception::Breakpoint => breakpoint::handler(e, tf),
        _ => panic!("unexpected exception"),
    }
}
