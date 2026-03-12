//! Page table type for Sv39 paging.

use alloc::alloc::{Layout, alloc_zeroed, dealloc};
use core::mem::size_of;

use super::addr::{PhysicalAddr, VirtualAddr};
use super::entry::Entry;
use super::{ENTRIES_PER_PAGE, PAGE_SIZE};
use crate::utils::{align_down, align_up};

#[derive(Debug)]
#[repr(transparent)]
pub struct Table {
    pub entries: [Entry; ENTRIES_PER_PAGE],
}
const_assert_eq!(size_of::<Table>(), PAGE_SIZE);

unsafe impl Send for Table {}

impl Table {
    pub const fn new() -> Self { Table { entries: [Entry::new(0); ENTRIES_PER_PAGE] } }

    /// Translate a virtual address to a physical address (read-only traversal).
    ///
    /// Returns `None` if any level of the page table walk encounters an invalid entry.
    pub fn translate(root: *const Self, vaddr: VirtualAddr) -> Option<PhysicalAddr> {
        let mut table = root;
        for lvl in (1..=2).rev() {
            let entry = unsafe { &(*table).entries[vaddr.extract_vpn(lvl)] };
            if !entry.is_valid() {
                return None;
            }
            let ppn = entry.extract_ppn_all();
            table = PhysicalAddr::from(ppn, 0).as_ptr::<Self>();
        }

        let entry = unsafe { &(*table).entries[vaddr.extract_vpn(0)] };
        if !entry.is_valid() {
            return None;
        }
        let ppn = entry.extract_ppn_all();
        Some(PhysicalAddr::from(ppn, vaddr.extract_offset()))
    }

    /// Walk to the leaf entry for a virtual address, allocating intermediate tables as needed.
    ///
    /// # Safety
    ///
    /// - `root` must be a valid pointer to a page table.
    pub unsafe fn walk_to_leaf(root: *mut Self, vaddr: VirtualAddr) -> &'static mut Entry {
        let mut table = root;
        for lvl in (1..=2).rev() {
            // SAFETY: caller guarantees root is valid and we only follow valid entries
            let entry = unsafe { &mut (*table).entries[vaddr.extract_vpn(lvl)] };
            if !entry.is_valid() {
                // SAFETY: Layout::new::<Table>() is valid and non-zero sized
                let new_table = unsafe { alloc_zeroed(Layout::new::<Table>()) } as *mut Table;
                entry.set_ppn(PhysicalAddr::new(new_table as usize));
                entry.set_valid();
            }
            let ppn = entry.extract_ppn_all();
            table = PhysicalAddr::from(ppn, 0).as_mut_ptr::<Self>();
        }
        // SAFETY: caller guarantees root is valid and we only follow valid entries
        unsafe { &mut (*table).entries[vaddr.extract_vpn(0)] }
    }

    /// Recursively visit all valid entries in the page table tree.
    ///
    /// The visitor is called for each valid entry with the entry reference and a boolean
    /// indicating whether it's a leaf entry. The visitor is called in post-order for branch
    /// entries (children visited first), which is useful for deallocation.
    ///
    /// # Safety
    ///
    /// - `root` must be a valid pointer to a page table.
    /// - The visitor must not invalidate pointers that will be traversed.
    pub unsafe fn visit_all<F>(root: *mut Self, visitor: &mut F)
    where
        F: FnMut(&mut Entry, bool),
    {
        // SAFETY: caller guarantees root is valid
        for entry in unsafe { (*root).entries.iter_mut() } {
            if entry.is_valid() {
                let is_leaf = entry.is_leaf();
                if entry.is_branch() {
                    let ppn = entry.extract_ppn_all();
                    let child = PhysicalAddr::from(ppn, 0).as_mut_ptr::<Self>();
                    // SAFETY: we only recurse into valid branch entries
                    unsafe { Self::visit_all(child, visitor) };
                }
                visitor(entry, is_leaf);
            }
        }
    }

    /// Recursively deallocate all intermediate page tables under root.
    ///
    /// This does not deallocate the root table itself, only its children.
    ///
    /// # Safety
    ///
    /// - `root` must be a valid pointer to a page table.
    /// - All intermediate tables must have been allocated with the global allocator.
    pub unsafe fn dealloc_all(root: *mut Self) {
        // SAFETY: caller guarantees root is valid
        unsafe {
            Self::visit_all(root, &mut |entry, is_leaf| {
                if !is_leaf {
                    let ppn = entry.extract_ppn_all();
                    // SAFETY: entry points to a table allocated with alloc_zeroed
                    dealloc(PhysicalAddr::from(ppn, 0).as_mut_ptr::<u8>(), Layout::new::<Table>());
                }
            });
        }
    }

    /// Map a single page: vaddr -> paddr with given flags.
    ///
    /// # Safety
    ///
    /// - `root` must be a valid pointer to a page table.
    pub unsafe fn map(root: *mut Self, vaddr: VirtualAddr, paddr: PhysicalAddr, flags: usize) {
        let entry = unsafe { Self::walk_to_leaf(root, vaddr) };
        entry.set_ppn(paddr);
        entry.set_flags(flags);
        entry.set_valid();
    }

    /// Map a range of pages using a custom address translation function.
    ///
    /// # Safety
    ///
    /// - `root` must be a valid pointer to a page table.
    pub unsafe fn map_range<F: Fn(VirtualAddr) -> PhysicalAddr>(
        root: *mut Self,
        start: usize,
        end: usize,
        f: F,
        flags: usize,
    ) {
        let start = align_down(start, PAGE_SIZE);
        let end = align_down(end, PAGE_SIZE);
        for curr in (start..end).step_by(PAGE_SIZE) {
            let vaddr = VirtualAddr::new(curr);
            let paddr = f(vaddr);
            unsafe { Self::map(root, vaddr, paddr, flags) };
        }
    }

    /// Identity map a range (vaddr == paddr).
    ///
    /// # Safety
    ///
    /// - `root` must be a valid pointer to a page table.
    pub unsafe fn id_map_range(root: *mut Self, start: usize, end: usize, flags: usize) {
        unsafe {
            Self::map_range(root, start, end, |v| PhysicalAddr::new(v.extract_bits()), flags)
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sv39::ENTRY_SIZE;

    #[test]
    fn test_table_size() {
        assert_eq!(size_of::<Table>(), PAGE_SIZE);
        assert_eq!(size_of::<Entry>(), ENTRY_SIZE);
        assert_eq!(ENTRIES_PER_PAGE, 512);
    }

    #[test]
    fn test_table_new_is_zeroed() {
        let table = Table::new();
        for i in 0..ENTRIES_PER_PAGE {
            assert!(!table.entries[i].is_valid());
        }
    }
}
