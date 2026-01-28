//! Sv39 paging kernel integration.
//!
//! This module re-exports types from the `paging` crate and provides
//! kernel-specific functions that require allocator access and raw pointer operations.

pub use paging::sv39::*;

use alloc::alloc::{alloc_zeroed, dealloc, Layout};

/// Allocate a zeroed page table and set the entry to point to it.
unsafe fn alloc_entry_page(entry: &mut Entry) {
    let ptr = alloc_zeroed(Layout::new::<Table>());

    entry.set_ppn(PhysicalAddr::new(ptr as usize));
    entry.set_valid();
}

/// Map a virtual address to a physical address in the page table.
///
/// The flags should contain only the following:
///   Read, Write, Execute, User, and/or Global
/// The flags MUST include one or more of the following:
///   Read, Write, Execute
pub unsafe fn map(root: *mut Table, vaddr: VirtualAddr, paddr: PhysicalAddr, flags: usize) {
    let mut table = root;
    for lvl in (1..=2).rev() {
        let entry = &mut (*table).entries[vaddr.extract_vpn(lvl)];
        if !entry.is_valid() {
            alloc_entry_page(entry);
        }

        let ppn = entry.extract_ppn_all();
        table = PhysicalAddr::from(ppn, 0).as_mut_ptr::<Table>();
    }

    let entry = &mut (*table).entries[vaddr.extract_vpn(0)];
    entry.set_ppn(paddr);
    entry.set_flags(flags);
    entry.set_valid();

    let mapped = virt_to_phys(root, vaddr);
    assert!(mapped == Some(paddr), "expect {:?} mapped to {:?} but get {:?}", vaddr, paddr, mapped);
}

/// Recursively unmap and deallocate all page tables under root.
pub unsafe fn unmap(root: *mut Table) {
    for entry in (*root).entries.iter_mut() {
        let ppn = entry.extract_ppn_all();
        if entry.is_valid() {
            if entry.is_branch() {
                let table = PhysicalAddr::from(ppn, 0).as_mut_ptr::<Table>();
                unmap(table);
            }
            dealloc(PhysicalAddr::from(ppn, 0).as_mut_ptr::<u8>(), Layout::new::<Table>());
        }
    }
}

/// Translate a virtual address to a physical address using the page table.
pub fn virt_to_phys(root: *const Table, vaddr: VirtualAddr) -> Option<PhysicalAddr> {
    let mut table = root;
    for lvl in (1..=2).rev() {
        let entry = unsafe { &(*table).entries[vaddr.extract_vpn(lvl)] };
        if !entry.is_valid() {
            return None;
        }
        let ppn = entry.extract_ppn_all();
        table = PhysicalAddr::from(ppn, 0).as_mut_ptr::<Table>();
    }

    let entry = unsafe { &(*table).entries[vaddr.extract_vpn(0)] };
    let ppn = entry.extract_ppn_all();
    Some(PhysicalAddr::from(ppn, vaddr.extract_offset()))
}
