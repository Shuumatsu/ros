use core::arch::asm;

use riscv::interrupt::supervisor::Interrupt;
use riscv::register::{sie, sstatus};

use crate::trap::TrapFrame;

mod clint;
mod plic;

pub unsafe fn init() {
    unsafe {
        println!("delegate all interrupts to supervisor mode");
        asm!("li t0, 0xffff");
        asm!("csrw mideleg, t0");

        sie::set_sext();
        sie::set_stimer();
        sie::set_ssoft();

        clint::init();
        plic::init();

        println!("enable supervisor interrupts");
        sstatus::set_sie();
    }
}

pub unsafe fn handler(intr: Interrupt, tf: &mut TrapFrame) {
    match intr {
        Interrupt::SupervisorTimer => clint::timer::handler(tf),
        Interrupt::SupervisorExternal => plic::handler(tf),
        _ => unimplemented!(),
    }
}
