//! RISC-V architecture support.

use mmu::VirtualAddr;

pub(crate) mod boot;
pub mod context;
pub mod interrupts;
pub mod sbi;
pub mod timer;
pub mod tlb;
pub mod trap;
pub mod user_access;

/// Cache-block size guaranteed by `Zic64b` and used for per-hart alignment.
pub const CACHE_LINE_BYTES: usize = 64;

/// A virtual address in the caller's instruction stream.
#[inline(always)]
pub fn pc() -> VirtualAddr {
    let pc: usize;
    // SAFETY: `auipc` with a zero offset computes this instruction's own address.
    unsafe { core::arch::asm!("auipc {}, 0", out(reg) pc, options(nomem, nostack)) };
    VirtualAddr::new(pc)
}

#[inline(always)]
pub fn sp() -> VirtualAddr {
    let sp: usize;
    // SAFETY: reading a register.
    unsafe { core::arch::asm!("mv {}, sp", out(reg) sp, options(nomem, nostack)) };
    VirtualAddr::new(sp)
}

/// Returns raw `tp`; zero means this hart has not adopted a control block.
#[inline(always)]
pub fn thread_pointer() -> usize {
    let tp: usize;
    // SAFETY: reading a register.
    unsafe { core::arch::asm!("mv {}, tp", out(reg) tp, options(nomem, nostack)) };
    tp
}

/// # Safety
///
/// `value` must have the representation and lifetime expected by every `tp` reader.
#[inline(always)]
pub unsafe fn set_thread_pointer(value: usize) {
    // SAFETY: forwarded from this function's contract.
    unsafe { core::arch::asm!("mv tp, {}", in(reg) value, options(nomem, nostack)) };
}

/// Reads the free-running `time` CSR.
#[inline(always)]
pub fn time_counter() -> u64 {
    let ticks: u64;
    // SAFETY: `rdtime` reads a counter and has no side effects.
    unsafe { core::arch::asm!("rdtime {}", out(reg) ticks, options(nomem, nostack)) };
    ticks
}

/// Waits until this hart takes an interrupt.
#[inline(always)]
pub fn idle() { riscv::asm::wfi() }

#[inline(always)]
pub fn wait_forever() -> ! {
    // SAFETY: no subsequent progress is required after entering this terminal state.
    unsafe { interrupts::disable() };
    loop {
        riscv::asm::wfi();
    }
}
