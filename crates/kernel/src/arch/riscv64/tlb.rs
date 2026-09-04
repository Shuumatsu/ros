//! Address-translation maintenance.
//!
//! RISC-V may cache missing translations. Each fence affects only the calling hart.

use mmu::Satp;

use super::interrupts;

/// Installs `satp` and flushes this hart's TLB atomically with respect to interrupts.
///
/// # Safety
///
/// `satp` must map the running PC and stack to their current physical addresses.
pub unsafe fn install(satp: Satp) {
    let bits = satp.bits();
    interrupts::without(|| {
        // SAFETY: forwarded from this function's contract.
        unsafe {
            core::arch::asm!(
                "csrw satp, {satp}",
                "sfence.vma",
                satp = in(reg) bits,
                options(nostack)
            );
        }
    });
}

/// Flushes all translations cached by this hart, including global entries.
pub fn flush_all() { riscv::asm::sfence_vma_all() }
