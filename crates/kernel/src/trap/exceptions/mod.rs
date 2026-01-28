use core::arch::asm;

use riscv::interrupt::supervisor::Exception;

use crate::trap::TrapFrame;

mod breakpoint;

pub unsafe fn init() {
    unsafe {
        println!("delegate all exceptions to supervisor mode");
        asm!("li t0, 0xffff");
        asm!("csrw medeleg, t0");
    }
}

pub fn handler(e: Exception, tf: &mut TrapFrame) {
    match e {
        Exception::Breakpoint => breakpoint::handler(e, tf),
        _ => panic!("unexpected exception"),
    }
}
