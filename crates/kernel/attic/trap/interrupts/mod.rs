use riscv::interrupt::supervisor::Interrupt;
use riscv::register::{sie, sstatus};

use crate::trap::TrapFrame;

mod clint;
mod plic;

pub unsafe fn init() {
    unsafe {
        // Delegation (`mideleg`) is the SBI firmware's job in M-mode. Bring up
        // the supervisor timer: arm the first tick via SBI, enable the S-timer
        // source, then unmask supervisor interrupts globally. Now that the
        // handler re-arms `set_timer` (which clears the pending bit), the Sstc
        // "always pending at stimecmp=0" trap that bit us in Phase 0 is gone.
        clint::timer::init();
        sie::set_stimer();
        sstatus::set_sie();

        // TODO: plic::init() + sie::set_sext() for external interrupts.
    }
}

pub unsafe fn handler(intr: Interrupt, tf: &mut TrapFrame) {
    match intr {
        Interrupt::SupervisorTimer => clint::timer::handler(tf),
        Interrupt::SupervisorExternal => unsafe { plic::handler(tf) },
        _ => unimplemented!(),
    }
}
