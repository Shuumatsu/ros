//! Supervisor trap decoding and vector management.

mod frame;

use core::fmt;

use riscv::interrupt::Trap;
use riscv::interrupt::supervisor::{Exception, Interrupt};
use riscv::register::{scause, scounteren, sepc, sscratch, stval, stvec};

use mmu::{MemoryAddr, VirtualAddr};

pub use frame::{TrapFrame, resume};

pub enum Cause {
    Timer,
    Software,
    External,
    Syscall,
    Fault(Fault),
}

pub struct Fault {
    scause: usize,
    exception: Option<Exception>,
    sepc: usize,
    stval: usize,
}

impl Fault {
    fn current(scause: usize, exception: Option<Exception>) -> Self {
        Self { scause, exception, sepc: sepc::read(), stval: stval::read() }
    }
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.exception {
            Some(exception) => write!(f, "{exception:?}")?,
            None => write!(f, "unknown cause (scause {:#x})", self.scause)?,
        }
        write!(f, " at sepc {:#018x}, stval {:#018x}", self.sepc, self.stval)
    }
}

pub fn vector() -> VirtualAddr { frame::vector() }

/// Installs the kernel trap vector on this hart.
pub fn install() {
    let vector = vector();
    assert!(
        vector.is_aligned(4),
        "trap vector {vector:#x} is not 4-byte aligned: stvec reads its low two bits as a mode"
    );

    // SAFETY: the kernel's own trap entry, in `.text`, which the page table this hart runs
    // on maps executable.
    unsafe { stvec::write(stvec::Stvec::new(vector.bits(), stvec::TrapMode::Direct)) };
}

/// # Safety
///
/// `control_block` must be the calling hart's valid, permanent [`Cpu`](crate::cpu::Cpu).
pub unsafe fn set_control_block(control_block: usize) {
    // SAFETY: forwarded from this function's contract.
    unsafe { sscratch::write(control_block) };
}

/// Allows user mode to read cycle, time, and instruction-retirement counters.
pub fn allow_user_counters() {
    let mut counters = scounteren::Scounteren::from_bits(0);
    counters.set_cy(true);
    counters.set_tm(true);
    counters.set_ir(true);
    // SAFETY: the counters are read-only to user mode and reading one has no side effect.
    unsafe { scounteren::write(counters) };
}

fn cause() -> Cause {
    let scause = scause::read();
    let decoded: Result<Trap<Interrupt, Exception>, _> = scause.cause().try_into();
    match decoded {
        Ok(Trap::Interrupt(Interrupt::SupervisorTimer)) => Cause::Timer,
        Ok(Trap::Interrupt(Interrupt::SupervisorSoft)) => Cause::Software,
        Ok(Trap::Interrupt(Interrupt::SupervisorExternal)) => Cause::External,
        Ok(Trap::Exception(Exception::UserEnvCall)) => Cause::Syscall,
        Ok(Trap::Exception(exception)) => {
            Cause::Fault(Fault::current(scause.bits(), Some(exception)))
        }
        Err(_) => Cause::Fault(Fault::current(scause.bits(), None)),
    }
}

/// # Safety
///
/// `frame` must be the frame created by the active trap on this hart.
unsafe extern "C" fn dispatch(frame: &mut TrapFrame) { crate::trap::handle(cause(), frame) }
