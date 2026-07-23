use core::arch::asm;

use riscv::register::*;

use crate::arch::riscv64 as arch;
use crate::cpu;
use crate::memory;
use crate::trap;

// static mut KERNEL_STARTED: bool = false;

#[unsafe(no_mangle)]
unsafe extern "C" fn start(dtb: usize) {
    let hart = arch::hart_id();
    if hart == 0 {
        // Discover hardware from the DTB the previous stage handed us in a1,
        // BEFORE anything prints: the console itself learns the UART base from
        // here. Zero-allocation, so it is safe to run before the heap exists.
        unsafe { crate::device_tree::discover(dtb) };
        cpu::print_info();
        crate::device_tree::dump();
    }

    println!("initializing memory...");
    memory::init();
    println!("initializing memory completed");

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

    // Configure PMP to allow S-mode access to all physical memory.
    // Without PMP entries, S-mode has no memory access on RISC-V.
    // NAPOT mode with all address bits set = match entire address space.
    // pmpcfg0[7:0] = 0x1f: R=1, W=1, X=1, A=NAPOT(11), L=0
    unsafe {
        asm!(
            "li {tmp}, 0x3fffffffffffff",
            "csrw pmpaddr0, {tmp}",
            "li {tmp}, 0x1f",
            "csrw pmpcfg0, {tmp}",
            tmp = out(reg) _,
        );
    }

    println!("switching to supervisor mode...");
    unsafe { asm!("mret") };

    unreachable!();
}

#[repr(align(4096))]
struct UserStack([u8; 4096]);
static USER_STACK: UserStack = UserStack([0u8; 4096]);

#[unsafe(no_mangle)]
unsafe extern "C" fn kmain() -> ! {
    kprintln!("enter kmain");

    println!("This is my operating system!");

    // Launch a hardcoded user-space program to demonstrate U-mode ecall handling.
    let entry = &crate::user_program::USER_PROGRAM as *const _ as usize;
    let user_sp = USER_STACK.0.as_ptr_range().end as usize;
    println!("jumping to user program at {:#x}, user sp = {:#x}", entry, user_sp);
    unsafe { trap::run_user_program(entry, user_sp) };

    let mut i = 0;
    while i < 10 {
        unsafe { trap::run_user_program(entry, user_sp) };
        i += 1;
    }

    unreachable!();
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
