use core::arch::asm;

use riscv::register::*;

use crate::arch::riscv64 as arch;
use crate::cpu;
use crate::trap;

// static mut KERNEL_STARTED: bool = false;
static INTERVAL: u64 = 10_0000;

#[unsafe(no_mangle)]
unsafe extern "C" fn start() {
    // disable paging for now.
    unsafe { satp::set(satp::Mode::Bare, 0, 0) };

    let hart = arch::hart_id();
    if hart == 0 {
        cpu::print_info();
    }

    println!("initializing traps...");
    unsafe { trap::init() };
    println!("initializing traps completed");

    println!("setting csrs for switching to supervisor mode...");
    // next mode is supervisor mode
    unsafe { mstatus::set_mpp(mstatus::MPP::Supervisor) };

    // mret jump to kmain or kmain_ap
    let main = if arch::hart_id() == 0 { kmain } else { kmain_ap };
    println!("setting mepc to main at {:#x}...", main as usize);
    unsafe { mepc::write(main as usize) };

    println!("switching to supervisor mode...");
    unsafe { asm!("mret") };

    unreachable!();
}

#[unsafe(no_mangle)]
unsafe extern "C" fn kmain() -> ! {
    crate::memory::init();
    println!("enter kmain");
    // println!("initializing paging...");
    // memory::paging::init();
    // println!("initializing paging completed");

    // KERNEL_STARTED = true;

    println!("This is my operating system!");

    unsafe { asm!("ebreak", options(nomem, nostack)) };

    // crate::echo::echo();

    scheduler();
}

#[unsafe(no_mangle)]
// mark the function as extern "C" to tell the compiler that it should use the C calling convention for this function
unsafe extern "C" fn kmain_ap() -> ! {
    println!("enter kmain_ap");

    // while !KERNEL_STARTED {}

    // println!("initializing paging...");
    // memory::paging::init();
    // println!("initializing paging completed");

    scheduler();
}

fn scheduler() -> ! {
    loop {
        unsafe {
            asm!("ebreak", options(nomem, nostack));
        }
    }
}
