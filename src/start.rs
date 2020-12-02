use core::mem::size_of;

use arch::{paging::sv39::virt_to_phys, stack_pointer};
use riscv::register::*;

use crate::arch::riscv64 as arch;
use crate::interrupt;
use crate::machine;
use crate::memory;
use crate::{kprint, kprintln, print, println};


#[no_mangle]
unsafe extern "C" fn start() {
    if arch::hart_id() == 0 {
        machine::print_info();

        kprintln!("initializing interrupts...");
        interrupt::init();

        kprintln!("initializing paging...");
        memory::paging::init();

        kprintln!("setting csrs for switching to supervisor mode...");
        // next mode is supervisor mode
        mstatus::set_mpp(mstatus::MPP::Supervisor);
        // mret jump to kmain
        mepc::write(kmain as usize);

        kprintln!("switching to supervisor mode...");
        asm!("mret");


        assert!(false, "should not be able to reach here");
    } else {
        kmain_ap();
    }
}

#[no_mangle]
unsafe extern "C" fn kmain() {
    println!("This is my operating system!");
    println!("I'm so awesome. If you start typing something, I'll show you what you typed!");

    crate::echo::echo();
}

#[no_mangle]
// mark the function as extern "C" to tell the compiler that it should use the C calling convention for this function
extern "C" fn kmain_ap() {
    kprintln!("ready and waiting for interrupts");

    arch::wait_forever();
}
