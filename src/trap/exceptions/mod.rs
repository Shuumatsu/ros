use riscv::register::{medeleg, scause::Exception};

use crate::trap::TrapFrame;
use crate::{print, println};

mod breakpoint;

pub unsafe fn init() {
    println!("delegate all exceptions to supervisor mode");
    asm!("li t0, 0xffff");
    asm!("csrw medeleg, t0");
}

pub fn handler(e: Exception, tf: &mut TrapFrame) {
    match e {
        Exception::Breakpoint => breakpoint::breakpoint(e, tf),
        _ => panic!("unexpected exception"),
    }
}
