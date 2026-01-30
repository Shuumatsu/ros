//! Sv39 paging kernel integration.
//!
//! This module re-exports types from the `paging` crate and provides
//! kernel-specific mapping functions.

pub use paging::sv39::*;

/// Map a virtual address to a physical address in the page table.
///
/// The flags should contain only the following:
///   Read, Write, Execute, User, and/or Global
/// The flags MUST include one or more of the following:
///   Read, Write, Execute
pub unsafe fn map(root: *mut Table, vaddr: VirtualAddr, paddr: PhysicalAddr, flags: usize) {
    // SAFETY: caller guarantees root is valid
    let entry = unsafe { Table::walk_to_leaf(root, vaddr) };
    entry.set_ppn(paddr);
    entry.set_flags(flags);
    entry.set_valid();

    let mapped = virt_to_phys(root, vaddr);
    assert!(mapped == Some(paddr), "expect {:?} mapped to {:?} but get {:?}", vaddr, paddr, mapped);
}

/// Recursively unmap and deallocate all page tables under root.
pub unsafe fn unmap(root: *mut Table) {
    // SAFETY: caller guarantees root is valid
    unsafe { Table::dealloc_all(root) };
}

/// Translate a virtual address to a physical address using the page table.
pub fn virt_to_phys(root: *const Table, vaddr: VirtualAddr) -> Option<PhysicalAddr> {
    Table::translate(root, vaddr)
}
