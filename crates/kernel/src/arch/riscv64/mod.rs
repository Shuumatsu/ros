//! RV64 privileged state: the instructions and CSRs the rest of the kernel reaches it
//! through.
//!
//! One module per CSR group that has a rule attached — [`interrupts`] owns `sstatus.sie`,
//! [`sbi`] the firmware calls, and [`tlb`] owns `satp` and `sfence.vma` for as long as
//! ordinary Rust is running. [`boot`] writes both before that, since installing the first
//! page table is what makes the kernel's own addresses resolve.
//!
//! Two ISA facts are read elsewhere, because the rule that governs them is what carries the
//! instruction: hart identity is `cpu`'s, whose `tp` holds a per-hart control block, and the
//! `time` counter is [`crate::time`]'s, which owns pairing ticks with the frequency that
//! gives them units.

use paging::VirtualAddr;

pub(crate) mod boot;
pub mod interrupts;
pub mod sbi;
pub mod tlb;

/// An address in the caller's instruction stream.
///
/// Always inlined, so the answer names the caller's `.text` rather than this function's.
/// Virtual: every caller runs with translation on, the boot stage having installed a table
/// before the first Rust frame.
#[inline(always)]
pub fn pc() -> VirtualAddr {
    let pc: usize;
    // SAFETY: `auipc` with a zero offset computes this instruction's own address.
    unsafe { core::arch::asm!("auipc {}, 0", out(reg) pc, options(nomem, nostack)) };
    VirtualAddr::new(pc)
}

/// This hart's stack pointer, inside the caller's frame for [`pc`]'s reason.
#[inline(always)]
pub fn sp() -> VirtualAddr {
    let sp: usize;
    // SAFETY: reading a register.
    unsafe { core::arch::asm!("mv {}, sp", out(reg) sp, options(nomem, nostack)) };
    VirtualAddr::new(sp)
}

/// Park this hart for good. The one parking primitive: `abort` and both `kmain`s
/// call it rather than open-coding the loop.
#[inline(always)]
pub fn wait_forever() -> ! {
    loop {
        riscv::asm::wfi();
    }
}
