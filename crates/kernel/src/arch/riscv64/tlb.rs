//! Address-translation maintenance: where `satp` is written and `sfence.vma` issued once
//! ordinary Rust is running. The boot stage writes both itself, of necessity — the first
//! table is what makes the kernel's linked addresses resolve at all.
//!
//! Installing a leaf is half of a mapping. RISC-V permits a hart to cache the *absence* of a
//! translation as readily as its presence, so until a fence retires that entry the hardware
//! may go on faulting on an address the page table now maps. Every edit to a live tree ends
//! here; [`crate::memory::address_space::AddressSpace::edit`] is what makes that
//! unavoidable.
//!
//! Everything below acts on the calling hart alone — `sfence.vma` is not a broadcast. A
//! tree that is live on another hart needs that hart to fence too, which means an IPI and
//! the RFENCE SBI extension. Neither exists yet, and until something edits a tree while a
//! second hart runs it, neither is needed.

use paging::Satp;

use super::interrupts;

/// Make `satp` the live translation on this hart, and retire everything cached under the
/// tree it replaces.
///
/// Interrupts are masked across the pair, so no trap can observe translation switched but
/// the TLB still holding the old tree's entries.
///
/// # Safety
///
/// `satp` must name a tree that maps the calling hart's running PC and stack pointer to the
/// same physical addresses the live tree does. Otherwise the next instruction fetch faults
/// with no table left to diagnose it from.
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

/// Retire every translation this hart has cached, global entries included.
///
/// The blunt instrument, and the right one after editing the kernel's tree: a fence
/// narrowed to an ASID deliberately spares global entries, and kernel leaves are exactly
/// what will carry `G` once there is a second address space to keep them across.
pub fn flush_all() { riscv::asm::sfence_vma_all() }
