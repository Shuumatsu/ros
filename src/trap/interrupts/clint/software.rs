use riscv::register::sie;

use crate::trap::TrapFrame;
use crate::{print, println};

// each CPU has a separate source of software interrupts.
pub unsafe fn init() {
    println!("enable supervisor software interrupts");
    sie::set_ssoft();
}
