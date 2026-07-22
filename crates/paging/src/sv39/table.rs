//! The Sv39 page-table type and the walk/map/translate operations over it.
//!
//! # Addressing model
//!
//! Table memory is allocated from the global allocator and its address is
//! stored in parent entries as a physical page number. Following an entry
//! therefore reinterprets that physical address as a pointer. This is only
//! sound while table memory is **directly addressable** — i.e. the kernel runs
//! identity-mapped (or otherwise maps table frames at their physical address),
//! which is the case before and after `satp` is switched to a table built here.

use alloc::alloc::{Layout, alloc_zeroed, dealloc, handle_alloc_error};
use core::mem::{align_of, size_of};

use super::addr::{MemoryAddr, PhysicalAddr, VirtualAddr};
use super::entry::{Entry, PteFlags};
use super::{ENTRIES_PER_PAGE, LEVELS, PAGE_OFFSET_BITS, PAGE_SIZE, VPN_BITS};
use crate::utils::{align_down, align_up, mask};

/// A single Sv39 page table: 512 entries filling exactly one 4 KiB frame.
///
/// The `align(4096)` is load-bearing: an entry stores `addr >> 12`, so a table
/// that is not page-aligned would have the low bits of its address silently
/// dropped and the walk would follow a different frame than was allocated.
#[derive(Debug)]
#[repr(C, align(4096))]
pub struct Table {
    pub entries: [Entry; ENTRIES_PER_PAGE],
}

const_assert_eq!(size_of::<Table>(), PAGE_SIZE);
const_assert_eq!(align_of::<Table>(), PAGE_SIZE);
// The `align(4096)` attribute above needs a literal; keep it honest.
const_assert_eq!(PAGE_SIZE, 4096);

/// Reinterpret a branch entry's target frame as a pointer to the child table.
#[inline]
fn child_table(entry: Entry) -> *mut Table {
    entry.target().as_mut_ptr::<Table>()
}

/// Allocate a fresh, zeroed page table and return a pointer to it.
///
/// Zeroed memory is a table full of invalid entries, which is exactly the
/// empty state we want. Aborts via `handle_alloc_error` if the allocator is
/// exhausted, matching the behaviour of `Box`.
fn alloc_table() -> *mut Table {
    let layout = Layout::new::<Table>();
    // SAFETY: `Table` has non-zero size (one page), so the layout is valid.
    let ptr = unsafe { alloc_zeroed(layout) } as *mut Table;
    if ptr.is_null() {
        handle_alloc_error(layout);
    }
    ptr
}

/// Reconstruct the physical address a leaf at `level` maps `vaddr` to.
///
/// The upper bits come from the entry's PPN; the low `12 + 9*level` bits (the
/// offset within a 4 KiB / 2 MiB / 1 GiB page) come from `vaddr`.
#[inline]
fn leaf_to_phys(entry: Entry, vaddr: VirtualAddr, level: usize) -> PhysicalAddr {
    let page_bits = PAGE_OFFSET_BITS + VPN_BITS * level;
    let in_page = mask(page_bits);
    let base = entry.ppn() << PAGE_OFFSET_BITS;
    PhysicalAddr::new((base & !in_page) | (vaddr.bits() & in_page))
}

impl Table {
    pub const fn new() -> Self {
        Self { entries: [Entry::empty(); ENTRIES_PER_PAGE] }
    }

    /// Translate a virtual address, honouring leaf mappings at any level
    /// (4 KiB, 2 MiB or 1 GiB). Returns `None` if the walk hits an invalid
    /// entry or a malformed table (a branch where a leaf must be).
    pub fn translate(&self, vaddr: VirtualAddr) -> Option<PhysicalAddr> {
        let mut table: *const Table = self;
        for level in (0..LEVELS).rev() {
            // SAFETY: `table` is either `self` or a child reached through a
            // valid branch entry; such frames are directly addressable.
            let entry = unsafe { (*table).entries[vaddr.vpn(level)] };
            if !entry.is_valid() {
                return None;
            }
            if entry.is_leaf() {
                return Some(leaf_to_phys(entry, vaddr, level));
            }
            table = child_table(entry);
        }
        // Fell off the bottom without a leaf: a branch at level 0 is malformed.
        None
    }

    /// Walk to the level-0 entry for `vaddr`, allocating intermediate tables
    /// along the way, and return a mutable reference to it.
    fn walk_create(&mut self, vaddr: VirtualAddr) -> &mut Entry {
        let mut table: *mut Table = self;
        for level in (1..LEVELS).rev() {
            // SAFETY: `table` is `self` or a child from a valid branch entry.
            let entry = unsafe { &mut (*table).entries[vaddr.vpn(level)] };
            if !entry.is_valid() {
                // Write a clean branch entry: valid, no permissions, so stale
                // R/W/X bits can never turn an intermediate into a fake leaf.
                let child = alloc_table();
                let mut branch = Entry::empty();
                branch.set_ppn(PhysicalAddr::new(child as usize));
                branch.set_flags(PteFlags::VALID);
                *entry = branch;
            } else {
                debug_assert!(entry.is_branch(), "walk hit a leaf/superpage mid-way");
            }
            table = child_table(*entry);
        }
        // SAFETY: reached the level-0 table.
        unsafe { &mut (*table).entries[vaddr.vpn(0)] }
    }

    /// Map a single 4 KiB page `vaddr -> paddr` with the given permissions.
    ///
    /// `flags` must name at least one of R/W/X (a leaf) and must not be the
    /// reserved write-only combination; `VALID` is applied automatically.
    pub fn map(&mut self, vaddr: VirtualAddr, paddr: PhysicalAddr, flags: PteFlags) {
        debug_assert!(flags.is_leaf(), "a mapping needs at least one of R/W/X");
        debug_assert!(flags.is_legal_leaf(), "write-only is a reserved encoding");
        debug_assert!(vaddr.is_aligned(PAGE_SIZE), "vaddr must be page-aligned");
        debug_assert!(paddr.is_aligned(PAGE_SIZE), "paddr must be page-aligned");

        let mut leaf = Entry::empty();
        leaf.set_ppn(paddr);
        leaf.set_flags(flags | PteFlags::VALID);
        *self.walk_create(vaddr) = leaf;
    }

    /// Map every 4 KiB page overlapping `[start, end)`, deriving each physical
    /// address from `translate`. `end` is rounded up so a partial final page is
    /// still fully mapped.
    pub fn map_range<F>(&mut self, start: usize, end: usize, translate: F, flags: PteFlags)
    where
        F: Fn(VirtualAddr) -> PhysicalAddr,
    {
        let start = align_down(start, PAGE_SIZE);
        let end = align_up(end, PAGE_SIZE);
        let mut va = start;
        while va < end {
            let vaddr = VirtualAddr::new(va);
            self.map(vaddr, translate(vaddr), flags);
            va += PAGE_SIZE;
        }
    }

    /// Identity-map `[start, end)` (each virtual page to the equal physical page).
    pub fn id_map_range(&mut self, start: usize, end: usize, flags: PteFlags) {
        self.map_range(start, end, |v| PhysicalAddr::new(v.bits()), flags);
    }

    /// Recursively free every intermediate table beneath this one.
    ///
    /// Leaf-mapped frames are left untouched (they are not owned here) and the
    /// root table itself is not freed. Freed branch entries are cleared so the
    /// tree is left consistent.
    ///
    /// # Safety
    ///
    /// Every branch under `self` must point to a table allocated by
    /// [`alloc_table`] (i.e. built through this crate's mapping API).
    pub unsafe fn free_subtables(&mut self) {
        for entry in self.entries.iter_mut() {
            if entry.is_branch() {
                let child = child_table(*entry);
                // SAFETY: a branch always points to a table we allocated; free
                // its descendants first (post-order) before the table itself.
                unsafe {
                    (*child).free_subtables();
                    dealloc(child as *mut u8, Layout::new::<Table>());
                }
                *entry = Entry::empty();
            }
        }
    }
}

impl Default for Table {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sv39::ENTRY_SIZE;
    use alloc::boxed::Box;

    /// A root table on the heap. `Box` honours `Table`'s 4 KiB alignment, so
    /// this mirrors how the kernel allocates its root.
    fn root() -> Box<Table> {
        let table = Box::new(Table::new());
        assert!(
            table.as_ref() as *const Table as usize % PAGE_SIZE == 0,
            "boxed root table must be page-aligned"
        );
        table
    }

    #[test]
    fn layout_is_a_page() {
        assert_eq!(size_of::<Table>(), PAGE_SIZE, "table fills one page");
        assert_eq!(align_of::<Table>(), PAGE_SIZE, "table is page-aligned");
        assert_eq!(size_of::<Entry>(), ENTRY_SIZE);
    }

    #[test]
    fn new_table_is_empty() {
        let t = Table::new();
        assert!(t.entries.iter().all(|e| !e.is_valid()), "fresh table has no valid entries");
    }

    #[test]
    fn map_then_translate_roundtrip() {
        let mut t = root();
        // Distinct index at every level: vpn2=1, vpn1=2, vpn0=3.
        let va = VirtualAddr::new((1 << 30) | (2 << 21) | (3 << 12));
        let pa = PhysicalAddr::new(0x8020_1000);

        t.map(va, pa, PteFlags::READ_WRITE);
        assert_eq!(t.translate(va), Some(pa), "exact page translates back");

        // Offset within the page is carried through.
        let va_off = VirtualAddr::new(va.bits() + 0x123);
        let pa_off = PhysicalAddr::new(pa.bits() + 0x123);
        assert_eq!(t.translate(va_off), Some(pa_off), "in-page offset preserved");

        // SAFETY: all sub-tables were built by `map`.
        unsafe { t.free_subtables() };
    }

    #[test]
    fn unmapped_translates_to_none() {
        let mut t = root();
        let mapped = VirtualAddr::new(0x8000_0000);
        t.map(mapped, PhysicalAddr::new(0x8000_0000), PteFlags::READ_WRITE);

        assert_eq!(t.translate(VirtualAddr::new(0x9000_0000)), None, "unmapped VA is None");
        unsafe { t.free_subtables() };
    }

    #[test]
    fn map_range_covers_partial_final_page() {
        let mut t = root();
        // [0x1000, 0x3001): 0x3001 lands in the page based at 0x3000, so three
        // pages must be mapped. `align_down` on the end would have dropped it.
        t.id_map_range(0x1000, 0x3001, PteFlags::READ_WRITE);

        for page in [0x1000, 0x2000, 0x3000] {
            assert_eq!(
                t.translate(VirtualAddr::new(page)),
                Some(PhysicalAddr::new(page)),
                "page {page:#x} in range must be mapped",
            );
        }
        assert_eq!(t.translate(VirtualAddr::new(0x4000)), None, "page past the range is unmapped");
        assert_eq!(t.translate(VirtualAddr::new(0x0)), None, "page before the range is unmapped");

        unsafe { t.free_subtables() };
    }

    #[test]
    fn remap_overwrites_leaf() {
        let mut t = root();
        let va = VirtualAddr::new(0x4000_0000);
        t.map(va, PhysicalAddr::new(0x1000), PteFlags::READ);
        t.map(va, PhysicalAddr::new(0x2000), PteFlags::READ_WRITE);

        assert_eq!(t.translate(va), Some(PhysicalAddr::new(0x2000)), "second map wins");
        unsafe { t.free_subtables() };
    }
}
