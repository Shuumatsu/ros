//! Supervisor trap decoding and vector management.

mod frame;

use core::fmt;

use riscv::interrupt::Trap;
use riscv::interrupt::supervisor::{Exception, Interrupt};
use riscv::register::{scause, scounteren, sepc, sscratch, stval, stvec};

pub use frame::{TrapFrame, resume, vector};

/// What the hart trapped for. Fault detail is left in the CSRs for [`Fault::current`].
pub enum Cause {
    Timer,
    Software,
    External,
    Syscall,
    Fault,
}

/// The supervisor CSRs describing an exception, snapshotted for reporting.
pub struct Fault {
    scause: usize,
    sepc: usize,
    stval: usize,
}

impl Fault {
    /// Snapshots the exception this hart is handling.
    ///
    /// The CSRs hold until this hart takes another trap, and dispatch runs with interrupts
    /// masked.
    pub fn current() -> Self {
        Self { scause: scause::read().bits(), sepc: sepc::read(), stval: stval::read() }
    }

    fn exception(&self) -> Option<Exception> {
        match decode(scause::Scause::from_bits(self.scause)) {
            Ok(Trap::Exception(exception)) => Some(exception),
            _ => None,
        }
    }
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.exception() {
            Some(exception) => write!(f, "{exception:?}")?,
            None => write!(f, "unknown cause (scause {:#x})", self.scause)?,
        }
        write!(f, " at sepc {:#018x}, stval {:#018x}", self.sepc, self.stval)
    }
}

/// Installs the kernel trap vector on this hart.
///
/// # Panics
///
/// Panics if the vector is not 4-byte aligned, which `stvec` would read as a mode.
pub fn install() {
    let vector = vector();
    let stvec = stvec::Stvec::try_new(vector.bits(), stvec::TrapMode::Direct)
        .unwrap_or_else(|_| panic!("trap vector {vector:#x} is not 4-byte aligned"));

    // SAFETY: the kernel's own trap entry, in `.text`, which the page table this hart runs
    // on maps executable.
    unsafe { stvec::write(stvec) };
}

/// # Safety
///
/// `control_block` must be the calling hart's valid, permanent [`Cpu`](crate::cpu::Cpu).
pub(super) unsafe fn set_control_block(control_block: usize) {
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

/// Names the trap `scause` reports, or fails on a cause this ISA revision does not define.
fn decode(scause: scause::Scause) -> Result<Trap<Interrupt, Exception>, riscv::result::Error> {
    scause.cause().try_into()
}

fn cause() -> Cause {
    match decode(scause::read()) {
        Ok(Trap::Interrupt(Interrupt::SupervisorTimer)) => Cause::Timer,
        Ok(Trap::Interrupt(Interrupt::SupervisorSoft)) => Cause::Software,
        Ok(Trap::Interrupt(Interrupt::SupervisorExternal)) => Cause::External,
        Ok(Trap::Exception(Exception::UserEnvCall)) => Cause::Syscall,
        _ => Cause::Fault,
    }
}

/// # Safety
///
/// `frame` must be the frame created by the active trap on this hart.
unsafe extern "C" fn dispatch(frame: &mut TrapFrame) { crate::trap::handle(cause(), frame) }
