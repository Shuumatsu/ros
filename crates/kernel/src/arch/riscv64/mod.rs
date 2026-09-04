//! RISC-V architecture support.

use mmu::VirtualAddr;

pub(crate) mod boot;
pub mod context;
pub mod interrupts;
pub mod sbi;
pub mod timebase;
pub mod tlb;
pub mod trap;
pub mod user_access;

/// Cache-block size guaranteed by `Zic64b` and used for per-hart alignment.
pub const CACHE_LINE_BYTES: usize = 64;

/// Alignment the ABI requires of `sp`, and therefore of every stack top a context begins on.
pub const STACK_ALIGN: usize = 16;

/// The address of a naked entry point, which has no callable Rust signature.
pub fn address_of(entry: unsafe extern "custom" fn()) -> VirtualAddr {
    VirtualAddr::from_ptr_of(entry as *const ())
}

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

/// Makes `control_block` this hart's identity, in `tp` and in `sscratch`.
///
/// Trap entry finds the block in `sscratch` and republishes it in `tp`, so a hart that holds
/// two different values loses one of them at its next trap.
///
/// # Safety
///
/// `control_block` must have the representation and lifetime expected by every `tp` reader.
pub unsafe fn adopt_control_block(control_block: usize) {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        core::arch::asm!("mv tp, {}", in(reg) control_block, options(nomem, nostack));
        trap::set_control_block(control_block);
    }
}

/// Waits until this hart takes an interrupt.
#[inline(always)]
pub fn idle() { riscv::asm::wfi() }

pub fn wait_forever() -> ! {
    // SAFETY: no subsequent progress is required after entering this terminal state.
    unsafe { interrupts::disable() };
    loop {
        idle();
    }
}
