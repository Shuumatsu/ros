use riscv::register::{scause::Interrupt, sstatus};

use crate::trap::TrapFrame;
use crate::{print, println};

mod clint;
mod plic;

pub unsafe fn init() {
    println!("delegate all interrupts to supervisor mode");
    asm!("li t0, 0xffff");
    asm!("csrw mideleg, t0");

    clint::init();
    plic::init();

    println!("enable supervisor interrupts");
    sstatus::set_sie();
}

pub unsafe fn handler(intr: Interrupt, tf: &mut TrapFrame) {
    match intr {
        Interrupt::SupervisorTimer => clint::timer::handler(tf),
        Interrupt::SupervisorExternal => plic::handler(tf),
        _ => unimplemented!(),
    }
}
