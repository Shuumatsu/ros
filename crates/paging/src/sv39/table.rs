//! The Sv39 page-table type: one page of hardware-defined entries.
//!
//! `Table` is pure data, plus only those operations that touch **this table's
//! own entries** — no frame allocation, no way to reach another frame. That is
//! what keeps the type usable in a `const` initializer and in code running
//! before any allocator exists.
//!
//! Anything that descends into child tables needs the caller's frame source and
//! addressing policy, so it lives in [`super::mapper`] instead.

use core::mem::{align_of, size_of};

use super::addr::{PhysicalAddr, VirtualAddr};
use super::entry::{Entry, PteFlags};
use super::page_size_at;
use super::{ENTRIES_PER_PAGE, PAGE_SIZE, ROOT_LEVEL};

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

impl Table {
    /// An empty table: every entry invalid.
    pub const fn new() -> Self { Self { entries: [Entry::empty(); ENTRIES_PER_PAGE] } }

    /// Install a 1 GiB gigapage leaf directly in this root table.
    ///
    /// A root-level leaf has no intermediate tables beneath it, so this
    /// allocates nothing and needs no way to reach other frames. That makes it
    /// the one mapping operation usable in a `const` initializer and in early
    /// boot code — which is exactly what the boot page table is built from.
    ///
    /// For anything smaller, or for a tree that already exists, use
    /// [`super::mapper::Mapper`].
    ///
    /// # Panics
    ///
    /// If `vaddr` or `paddr` is not 1 GiB aligned, or `flags` is not a legal
    /// leaf. In a `const` context these are compile-time errors.
    pub const fn map_gigapage(&mut self, vaddr: VirtualAddr, paddr: PhysicalAddr, flags: PteFlags) {
        const GIGAPAGE: usize = page_size_at(ROOT_LEVEL);
        assert!(flags.is_leaf(), "a gigapage mapping needs at least one of R/W/X");
        assert!(flags.is_legal_leaf(), "write-without-read is a reserved PTE encoding");
        assert!(
            vaddr.bits() & (GIGAPAGE - 1) == 0,
            "a gigapage virtual address must be 1 GiB aligned"
        );
        assert!(
            paddr.bits() & (GIGAPAGE - 1) == 0,
            "a gigapage physical address must be 1 GiB aligned"
        );
        self.entries[vaddr.vpn(ROOT_LEVEL)] = Entry::leaf(paddr, flags);
    }
}

impl Default for Table {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sv39::ENTRY_SIZE;

    /// Bottom of the Sv39 high half, where a higher-half kernel lives.
    const HIGH_BASE: usize = 0xffff_ffc0_0000_0000;
    const GIGAPAGE: usize = 1 << 30;
    /// The permissions an early boot mapping needs: all access, and A/D
    /// pre-set so the hardware walker never has to write to the table.
    const BOOT: PteFlags =
        PteFlags::READ_WRITE_EXECUTE.union(PteFlags::ACCESS).union(PteFlags::DIRTY);

    /// Build a boot-style table **at compile time**: the full low canonical half
    /// identity mapped, and mirrored into the high half. Evaluating it here proves
    /// it costs nothing at run time and requires no allocator.
    const fn early_table() -> Table {
        let mut table = Table::new();
        let mut i = 0;
        while i < ENTRIES_PER_PAGE / 2 {
            let pa = PhysicalAddr::new(i * GIGAPAGE);
            table.map_gigapage(VirtualAddr::new(i * GIGAPAGE), pa, BOOT);
            table.map_gigapage(VirtualAddr::new(HIGH_BASE + i * GIGAPAGE), pa, BOOT);
            i += 1;
        }
        table
    }

    /// Forced through const evaluation: if `map_gigapage` were not truly
    /// const-usable, this would not compile.
    static EARLY: Table = early_table();

    #[test]
    fn layout_is_a_page() {
        assert_eq!(size_of::<Table>(), PAGE_SIZE, "table fills one page");
        assert_eq!(align_of::<Table>(), PAGE_SIZE, "table is page-aligned");
        assert_eq!(size_of::<Entry>(), ENTRY_SIZE);
    }

    #[test]
    fn new_table_is_empty() {
        let table = Table::new();
        assert!(table.entries.iter().all(|e| !e.is_valid()), "fresh table has no valid entries");
    }

    #[test]
    fn const_built_boot_table_has_the_expected_entries() {
        // The high half starts at root index 256 — derived, not assumed.
        let high_index = VirtualAddr::new(HIGH_BASE).vpn(ROOT_LEVEL);
        assert_eq!(high_index, 256, "high half begins at root entry 256");

        for i in 0..ENTRIES_PER_PAGE / 2 {
            let expected = PhysicalAddr::new(i * GIGAPAGE);

            let identity = EARLY.entries[i];
            assert!(identity.is_leaf(), "identity entry {i} must be a leaf, not a branch");
            assert_eq!(identity.target(), expected, "identity entry {i} targets the wrong frame");

            let high = EARLY.entries[high_index + i];
            assert!(high.is_leaf(), "high-half entry {i} must be a leaf");
            assert_eq!(high.target(), expected, "high-half entry {i} targets the wrong frame");
            assert_eq!(high.flags(), BOOT | PteFlags::VALID, "high-half entry {i} lost flags");
        }

        assert!(
            EARLY.entries.iter().all(|entry| entry.is_valid()),
            "the two canonical halves must fill the root table"
        );
    }

    #[test]
    fn gigapage_entry_encodes_ppn_and_flags() {
        let mut table = Table::new();
        let pa = PhysicalAddr::new(2 * GIGAPAGE);
        table.map_gigapage(VirtualAddr::new(3 * GIGAPAGE), pa, BOOT);

        let entry = table.entries[3];
        assert_eq!(entry.target(), pa, "entry must carry the mapped frame");
        assert!(entry.flags().contains(PteFlags::VALID), "VALID is applied automatically");
        assert!(entry.is_leaf(), "a gigapage is a leaf");
    }

    #[test]
    #[should_panic(expected = "1 GiB aligned")]
    fn rejects_a_misaligned_gigapage() {
        let mut table = Table::new();
        table.map_gigapage(VirtualAddr::new(0x1000), PhysicalAddr::new(0), BOOT);
    }
}
