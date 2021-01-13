use riscv::register::*;

use crate::arch::riscv64 as arch;
use crate::cpu;
use crate::memory::layout::{clint_mtimecmp, CLINT_MTIME};
use crate::trap;

// static mut KERNEL_STARTED: bool = false;
static INTERVAL: u64 = 10_0000;

#[no_mangle]
unsafe extern "C" fn start() {
    // disable paging for now.
    satp::set(satp::Mode::Bare, 0, 0);

    let hart = arch::hart_id();
    if hart == 0 {
        cpu::print_info();
    }

    println!("initializing traps...");
    trap::init();
    println!("initializing traps completed");

    println!("setting csrs for switching to supervisor mode...");
    // next mode is supervisor mode
    mstatus::set_mpp(mstatus::MPP::Supervisor);

    // mret jump to kmain or kmain_ap
    let main = if arch::hart_id() == 0 { kmain } else { kmain_ap };
    println!("setting mepc to main at {:#x}...", main as usize);
    mepc::write(main as usize);

    println!("switching to supervisor mode...");
    asm!("mret");

    unreachable!();
}

#[no_mangle]
unsafe extern "C" fn kmain() -> ! {
    println!("enter kmain");
    // println!("initializing paging...");
    // memory::paging::init();
    // println!("initializing paging completed");

    // KERNEL_STARTED = true;

    println!("This is my operating system!");

    llvm_asm!("ebreak"::::"volatile");

    // crate::echo::echo();

    scheduler();
}

#[no_mangle]
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
            llvm_asm!("ebreak"::::"volatile");
        }
    }
    arch::wait_forever()
}
