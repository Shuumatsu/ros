use riscv::register::{mie, mstatus, sie, sstatus};

use crate::arch::riscv64::hart_id;
use crate::memory::layout::{clint_mtime, clint_mtimecmp};
use crate::trap::TrapFrame;
use crate::{print, println};

// RISC-V要求在机器模式下处理定时器中断，而不是监督者模式
// RISC-V uses 2 memory-mapped registers mtime and mtimecmp to control timer interrupts.

pub static mut TICK: usize = 0;
static INTERVAL: u64 = 10_0000;

// each CPU has a separate source of timer interrupts.
pub unsafe fn init() {
    let hart = hart_id();
    set_timer(hart);

    mstatus::set_mie();
    mie::set_mtimer();
    mie::set_msoft();

    sstatus::set_sie();
    sie::set_stimer();
    sie::set_ssoft();
}

unsafe fn set_timer(hart: usize) {
    // Machine timer
    let mtimecmp = clint_mtimecmp(hart) as *mut u64;
    let mtime = clint_mtime(hart) as *const u64;

    mtimecmp.write_volatile(mtime.read_volatile() + INTERVAL);
}

// 当前时间加上 TIMEBASE 为下一次中断产生的时间
// pub fn clock_set_next_event() { set_timer(get_cycle() + INTERVAL); }

// fn super_timer(tf: &mut TrapFrame) {
//     let mtimecmp = 0x0200_4000 as *mut u64;
//     let mtime = 0x0200_bff8 as *const u64;
//     // The frequency given by QEMU is 10_000_000 Hz, so this sets
//     // the next interrupt to fire one second from now.
//     mtimecmp.write_volatile(mtime.read_volatile() + 10_000_000);
// }

pub fn handler(tf: &mut TrapFrame) { unimplemented!("stimer handler") }
