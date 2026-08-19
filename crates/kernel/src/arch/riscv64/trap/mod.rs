//! Where a supervisor trap lands, and what the CSRs say it was.
//!
//! Two halves. [`frame`] owns the vector, the register file it saves and the return that puts
//! one back; this file owns `stvec`, `scause`, `sepc` and `stval` — the CSRs that say a trap
//! happened and what kind — together with `sscratch`, which is how the vector finds the hart it
//! is running on, and `scounteren`, whose bits decide whether a counter read from user mode is a
//! trap at all. Neither half decides anything: [`crate::trap`] does, on a [`Cause`] this module
//! hands it, so the ISA layer names every trap the hardware can deliver and the kernel layer
//! names what each one costs.
//!
//! Direct mode, so every cause arrives at one entry. Vectored mode would spread the
//! interrupt causes over a table of jumps and buy nothing while the dispatch below is a
//! single `match`.

mod frame;

use core::fmt;

use riscv::interrupt::Trap;
use riscv::interrupt::supervisor::{Exception, Interrupt};
use riscv::register::{scause, scounteren, sepc, sscratch, stval, stvec};

use mmu::{MemoryAddr, VirtualAddr};

pub use frame::{TrapFrame, resume};

/// A trap, decoded into the terms the kernel dispatches on.
///
/// The three interrupts are named even though only the timer has a handler: what the
/// hardware can deliver is this layer's knowledge, and "software interrupt with nobody
/// listening" is a better report than a raw `scause`.
pub enum Cause {
    Timer,
    Software,
    External,
    /// A process asking the kernel for something: `ecall` from user mode.
    ///
    /// Its own variant rather than one of the [`Fault`]s, because it is the one exception that
    /// is not an error — the interrupted context resumes, one instruction along.
    Syscall,
    /// A trap the kernel has no way to resume from.
    Fault(Fault),
}

/// A fatal trap, with everything the CSRs say about it.
pub struct Fault {
    scause: usize,
    /// `None` for a cause outside the standard set, whose `scause` is then all there is.
    exception: Option<Exception>,
    sepc: usize,
    stval: usize,
}

impl Fault {
    /// Read the rest of what this hart says about the trap it is in.
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

/// The address this hart's `stvec` gets, and the one the log reports.
pub fn vector() -> VirtualAddr { frame::vector() }

/// Point this hart's `stvec` at the kernel's trap vector.
///
/// Per hart, because `stvec` is a CSR, and after the kernel page table is live, because the
/// vector is a kernel virtual address. Until it runs the hart still carries the boot stage's
/// park vector, which stops it dead on a trap instead of dispatching one.
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

/// Point this hart's `sscratch` at its control block.
///
/// Written once, when the hart adopts the block, and read by every trap after it. The vector
/// arrives with every register holding the interrupted context, so a CSR is the only place it can
/// find anything at all — which is why this is the copy that has to be right.
///
/// # Safety
///
/// `control_block` must be the calling hart's [`Cpu`](crate::cpu::Cpu), valid for as long as the
/// kernel runs. The vector spills a register into it and restores `tp` from it without being able
/// to check either.
pub unsafe fn set_control_block(control_block: usize) {
    // SAFETY: forwarded from this function's contract.
    unsafe { sscratch::write(control_block) };
}

/// Let user mode read the counters `rdcycle`, `rdtime` and `rdinstret` name.
///
/// Per hart, because `scounteren` is a CSR, and alongside the vector because both are what a hart
/// owes before user mode can run on it: without these bits one of those reads is an illegal
/// instruction rather than a read. Writes the whole register, so the counters this kernel has
/// never heard of stay off.
pub fn allow_user_counters() {
    let mut counters = scounteren::Scounteren::from_bits(0);
    counters.set_cy(true);
    counters.set_tm(true);
    counters.set_ir(true);
    // SAFETY: the counters are read-only to user mode and reading one has no side effect.
    unsafe { scounteren::write(counters) };
}

/// What just happened, as this hart's CSRs report it.
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

/// The only Rust the trap entry calls.
///
/// # Safety
///
/// Reached from [`frame`]'s vector with `a0` the frame it just filled on this hart's stack.
/// Nothing else may call it: the `&mut TrapFrame` is what `sret` will restore from.
unsafe extern "C" fn dispatch(frame: &mut TrapFrame) { crate::trap::handle(cause(), frame) }
