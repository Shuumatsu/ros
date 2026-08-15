//! RV64 privileged state: the instructions and CSRs the rest of the kernel reaches it
//! through.
//!
//! One module per CSR group that has a rule attached — [`interrupts`] owns `sstatus.sie`,
//! [`tlb`] owns `satp` and `sfence.vma`, [`sbi`] the firmware calls — so each rule has one
//! place to live. Hart identity is `cpu`'s: `tp` points at a per-hart control block, and
//! reading a field out of it is that module's business.

pub(crate) mod boot;
pub mod interrupts;
pub mod sbi;
pub mod tlb;

/// Park this hart for good. The one parking primitive: `abort` and both `kmain`s
/// call it rather than open-coding the loop.
#[inline(always)]
pub fn wait_forever() -> ! {
    loop {
        riscv::asm::wfi();
    }
}
