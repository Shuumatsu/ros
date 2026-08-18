//! RV64 privileged state: the instructions and CSRs the rest of the kernel reaches it
//! through.
//!
//! One module per CSR group that has a rule attached — [`interrupts`] owns `sstatus.sie`,
//! [`sbi`] the firmware calls, and [`tlb`] owns `satp` and `sfence.vma` for as long as
//! ordinary Rust is running. [`boot`] writes both before that, since installing the first
//! page table is what makes the kernel's own addresses resolve.
//!
//! The loose instructions below have no CSR group of their own. Each is a bare register
//! read or write whose *policy* belongs to a subsystem elsewhere: `cpu` decides that a
//! non-zero [`thread_pointer`] means a live control block, and [`crate::time`] decides what
//! a [`time_counter`] tick is worth. Splitting them that way is what keeps an `asm!` block
//! out of a module named after a concern rather than an instruction set.

use mmu::VirtualAddr;

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

/// `tp`, which this kernel reserves for [`crate::cpu`]'s per-hart pointer.
///
/// Raw bits, because what a value there *means* is `cpu`'s: the boot entry zeroes the
/// register so that zero can stand for "no control block adopted yet", and firmware leaves
/// garbage that is indistinguishable from a live pointer.
#[inline(always)]
pub fn thread_pointer() -> usize {
    let tp: usize;
    // SAFETY: reading a register.
    unsafe { core::arch::asm!("mv {}, tp", out(reg) tp, options(nomem, nostack)) };
    tp
}

/// Point `tp` at `value` on this hart.
///
/// # Safety
///
/// Nothing else may use `tp`, and every reader of [`thread_pointer`] must agree with the
/// caller about what `value` points at — a wrong pointer here is dereferenced as a control
/// block by code that cannot check it.
#[inline(always)]
pub unsafe fn set_thread_pointer(value: usize) {
    // SAFETY: forwarded from this function's contract.
    unsafe { core::arch::asm!("mv tp, {}", in(reg) value, options(nomem, nostack)) };
}

/// The `time` CSR: a free-running counter, readable in S-mode. The only read of it.
///
/// Counts at a rate this instruction does not carry; pairing it with the platform's
/// frequency is [`crate::time`]'s. S-mode access is granted by M-mode through
/// `mcounteren.TM`, and where firmware withholds it the instruction traps and is emulated —
/// which is why the read is here and not at its caller.
#[inline(always)]
pub fn time_counter() -> u64 {
    let ticks: u64;
    // SAFETY: `rdtime` reads a counter and has no side effects.
    unsafe { core::arch::asm!("rdtime {}", out(reg) ticks, options(nomem, nostack)) };
    ticks
}

/// Park this hart for good. The one parking primitive: `abort` and both `kmain`s
/// call it rather than open-coding the loop.
#[inline(always)]
pub fn wait_forever() -> ! {
    loop {
        riscv::asm::wfi();
    }
}
