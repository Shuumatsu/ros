use log::{debug, error};
use riscv::register::{
    mtvec::TrapMode,
    scause::{self, Exception, Trap},
    stval, stvec,
};

use trapframe::TrapFrame;

use crate::syscall::syscall;

pub fn init() {
    unsafe {
        trapframe::init();
    }
}

#[no_mangle]
pub fn trap_handler(cx: &mut TrapFrame) -> &mut TrapFrame {
    let scause = scause::read();
    let stval = stval::read();

    println!("{:?} {:#x} {:#x}", scause.cause(), stval, cx.sepc);

    match scause.cause() {
        Trap::Exception(Exception::UserEnvCall) => {
            cx.sepc += 4;
            cx.general.a0 =
                syscall(cx.general.a7, [cx.general.a0, cx.general.a1, cx.general.a2]) as usize;
        }
        Trap::Exception(Exception::StoreFault) | Trap::Exception(Exception::StorePageFault) => {
            error!("[kernel] StoreFault/StorePageFault in application, core dumped.");
        }
        Trap::Exception(Exception::LoadFault) | Trap::Exception(Exception::LoadPageFault) => {
            error!("[kernel] LoadFault/LoadPageFault in application, core dumped.");
            if stval == 0x951231 {
                cx.sepc += 4;
            } else {
                error!("[kernel] LoadFault/LoadPageFault in application, core dumped.");
            }
        }
        Trap::Exception(Exception::IllegalInstruction) => {
            error!("[kernel] IllegalInstruction in application, core dumped.");
        }
        _ => {
            panic!(
                "Unsupported trap {:?}, stval = {:#x}!",
                scause.cause(),
                stval
            );
        }
    }
    cx
}
