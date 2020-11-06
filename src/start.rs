#[cfg(riscv64)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Bare = 0,
    Sv39 = 8,
    Sv48 = 9,
    Sv57 = 10,
    Sv64 = 11,
}

#[cfg(riscv32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Bare = 0,
    Sv39 = 8,
    Sv48 = 9,
    Sv57 = 10,
    Sv64 = 11,
}

use riscv::register::*;

use crate::arch;
use crate::{print, println};

#[no_mangle]
unsafe extern "C" fn start() {
    // delegate all interrupts and exceptions to supervisor mode
    asm!("li t0, 0xffff");
    asm!("csrw medeleg, t0");
    asm!("li t0, 0xffff");
    asm!("csrw mideleg, t0");
    // supervisor interrupt enable
    sie::set_ssoft(); // software
    sie::set_sext(); // external
    sie::set_stimer(); // timer

    // next mode is supervisor mode
    mstatus::set_mpp(mstatus::MPP::Supervisor);

    // mret jump to kmain
    mepc::write(kernel as usize);

    asm!("mret");
}

#[no_mangle]
// mark the function as extern "C" to tell the compiler that it should use the C calling convention for this function
extern "C" fn kernel() {
    println!("{}: {}", arch::hart_id(), kernel as usize);
    if arch::hart_id() == 0 {
        println!("This is my operating system!");
        println!("I'm so awesome. If you start typing something, I'll show you what you typed!");

        crate::echo::echo();
    } else {
        arch::wait_forever();
    }
}
