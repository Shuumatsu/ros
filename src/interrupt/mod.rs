use riscv::register::*;

pub const CLINT_BASE_ADDR: usize = 0x0200_0000;

pub unsafe fn init() {
    // delegate all interrupts and exceptions to supervisor mode
    asm!("li t0, 0xffff");
    asm!("csrw medeleg, t0");
    asm!("li t0, 0xffff");
    asm!("csrw mideleg, t0");

    // supervisor interrupt enable
    sie::set_ssoft(); // software
    sie::set_sext(); // external
    sie::set_stimer(); // timer
}
