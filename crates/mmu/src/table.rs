//! A scheme-independent hardware page-table page and root-level builders.

use core::mem::{align_of, size_of};

use crate::addr::{PhysicalAddr, VirtualAddr};
use crate::geometry::{ENTRIES_PER_PAGE, PAGE_SIZE, ROOT_ENTRIES_PER_HALF};
use crate::pte::{Entry, PteFlags};
use crate::scheme::{Scheme, vpn};

/// A 512-entry, 4 KiB page table.
///
/// Page alignment is required because entries store frame addresses without
/// their low 12 bits.
#[derive(Debug)]
#[repr(C, align(4096))]
pub struct Table {
    pub entries: [Entry; ENTRIES_PER_PAGE],
}

const_assert_eq!(size_of::<Table>(), PAGE_SIZE);
const_assert_eq!(align_of::<Table>(), PAGE_SIZE);
const_assert_eq!(PAGE_SIZE, 4096);

impl Table {
    pub const fn new() -> Self { Self { entries: [Entry::empty(); ENTRIES_PER_PAGE] } }

    /// Share `from`'s upper-half root subtrees.
    ///
    /// Later mappings are shared only through root slots copied by this call.
    /// Both tables must be roots.
    pub fn share_upper_half(&mut self, from: &Table) {
        let upper = ENTRIES_PER_PAGE - ROOT_ENTRIES_PER_HALF;
        self.entries[upper..].copy_from_slice(&from.entries[upper..]);
    }

    /// Return the root entry selected by `vaddr`.
    pub fn root_slot<S: Scheme>(&self, vaddr: VirtualAddr) -> Entry {
        self.entries[vpn::<S>(vaddr, S::ROOT_LEVEL)]
    }

    /// Install an allocation-free root leaf covering `S::ROOT_PAGE` bytes.
    ///
    /// # Panics
    ///
    /// If either address is not root-page aligned or `flags` is not a legal leaf.
    pub const fn map_root_page<S: Scheme>(
        &mut self,
        vaddr: VirtualAddr,
        paddr: PhysicalAddr,
        flags: PteFlags,
    ) {
        assert!(flags.is_leaf(), "a root-page mapping needs at least one of R/W/X");
        assert!(flags.is_legal_leaf(), "write-without-read is a reserved PTE encoding");
        assert!(
            vaddr.bits() & (S::ROOT_PAGE - 1) == 0,
            "a root-page virtual address must be aligned to a whole root slot"
        );
        assert!(
            paddr.bits() & (S::ROOT_PAGE - 1) == 0,
            "a root-page physical address must be aligned to a whole root slot"
        );
        self.entries[vpn::<S>(vaddr, S::ROOT_LEVEL)] = Entry::leaf(paddr, flags);
    }

    /// Build an early root table with an identity-mapped low half and a direct
    /// map of `[0, offset_span)` at `va_offset`.
    ///
    /// Root slots above the direct-map window remain invalid. `flags` applies to
    /// every installed leaf.
    ///
    /// # Panics
    ///
    /// If `va_offset` is not the high-half base or `offset_span` is not a
    /// non-zero, root-page-aligned span that fits in that half.
    pub const fn identity_and_offset<S: Scheme>(
        va_offset: usize,
        offset_span: usize,
        flags: PteFlags,
    ) -> Self {
        let root_page = S::ROOT_PAGE;
        assert!(
            va_offset.is_multiple_of(root_page),
            "the high half must begin on a root-page boundary"
        );
        assert!(
            vpn::<S>(VirtualAddr::new(va_offset), S::ROOT_LEVEL) == ROOT_ENTRIES_PER_HALF,
            "va_offset must be the base of the high canonical half"
        );
        assert!(
            offset_span.is_multiple_of(root_page),
            "the offset half is built from root pages, so its span must be a multiple of one"
        );
        let offset_slots = offset_span / root_page;
        assert!(offset_slots > 0, "the offset half must map something");
        assert!(
            offset_slots <= ROOT_ENTRIES_PER_HALF,
            "the offset half cannot be wider than the canonical half holding it"
        );

        let mut table = Self::new();
        let mut index = 0;
        while index < ROOT_ENTRIES_PER_HALF {
            let pa = PhysicalAddr::new(index * root_page);
            table.map_root_page::<S>(VirtualAddr::new(index * root_page), pa, flags);
            if index < offset_slots {
                table.map_root_page::<S>(
                    VirtualAddr::new(va_offset + index * root_page),
                    pa,
                    flags,
                );
            }
            index += 1;
        }
        table
    }
}

impl Default for Table {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{ENTRY_SIZE, GIGAPAGE};
    use crate::scheme::{Sv39, Sv48, vpn};

    const HIGH_BASE: usize = 0xffff_ffc0_0000_0000;
    const SPAN: usize = (ROOT_ENTRIES_PER_HALF / 2) * GIGAPAGE;
    const BOOT: PteFlags =
        PteFlags::READ_WRITE_EXECUTE.union(PteFlags::ACCESS).union(PteFlags::DIRTY);

    static EARLY: Table = Table::identity_and_offset::<Sv39>(HIGH_BASE, SPAN, BOOT);

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
        let high_index = vpn::<Sv39>(VirtualAddr::new(HIGH_BASE), Sv39::ROOT_LEVEL);
        assert_eq!(high_index, ROOT_ENTRIES_PER_HALF, "high half begins at root entry 256");
        let window_slots = SPAN / GIGAPAGE;

        for i in 0..ROOT_ENTRIES_PER_HALF {
            let expected = PhysicalAddr::new(i * GIGAPAGE);

            let identity = EARLY.entries[i];
            assert!(identity.is_leaf(), "identity entry {i} must be a leaf, not a branch");
            assert_eq!(identity.target(), expected, "identity entry {i} targets the wrong frame");

            let high = EARLY.entries[high_index + i];
            if i >= window_slots {
                assert!(!high.is_valid(), "high-half entry {i} is outside the window and mapped");
                continue;
            }
            assert!(high.is_leaf(), "high-half entry {i} must be a leaf");
            assert_eq!(high.target(), expected, "high-half entry {i} targets the wrong frame");
            assert_eq!(high.flags(), BOOT | PteFlags::VALID, "high-half entry {i} lost flags");
        }

        let valid = EARLY.entries.iter().filter(|entry| entry.is_valid()).count();
        assert_eq!(
            valid,
            ROOT_ENTRIES_PER_HALF + window_slots,
            "the table must map the whole low half and exactly the window above it"
        );
    }

    #[test]
    fn a_full_width_offset_half_fills_the_table() {
        let table =
            Table::identity_and_offset::<Sv39>(HIGH_BASE, ROOT_ENTRIES_PER_HALF * GIGAPAGE, BOOT);
        assert!(
            table.entries.iter().all(|entry| entry.is_valid()),
            "a window as wide as the half must leave no slot invalid"
        );
    }

    #[test]
    fn the_builder_follows_the_scheme_it_is_given() {
        const SV48_HIGH_BASE: usize = 0xffff_8000_0000_0000;
        let span = 4 * Sv48::ROOT_PAGE;
        let table = Table::identity_and_offset::<Sv48>(SV48_HIGH_BASE, span, BOOT);

        let high_index = vpn::<Sv48>(VirtualAddr::new(SV48_HIGH_BASE), Sv48::ROOT_LEVEL);
        assert_eq!(high_index, ROOT_ENTRIES_PER_HALF, "Sv48's high half also begins at slot 256");
        assert_eq!(
            table.entries[high_index + 3].target(),
            PhysicalAddr::new(3 * Sv48::ROOT_PAGE),
            "the fourth high slot maps the fourth 512 GiB of physical memory"
        );
        assert!(!table.entries[high_index + 4].is_valid(), "the window stops after four slots");
        assert_eq!(
            table.entries.iter().filter(|entry| entry.is_valid()).count(),
            ROOT_ENTRIES_PER_HALF + 4,
            "the whole low half, and exactly the window above it"
        );
    }

    #[test]
    #[should_panic(expected = "multiple of one")]
    fn rejects_a_window_that_is_not_whole_root_pages() {
        let _ = Table::identity_and_offset::<Sv39>(HIGH_BASE, GIGAPAGE + PAGE_SIZE, BOOT);
    }

    #[test]
    #[should_panic(expected = "must map something")]
    fn rejects_an_empty_window() { let _ = Table::identity_and_offset::<Sv39>(HIGH_BASE, 0, BOOT); }

    #[test]
    #[should_panic(expected = "cannot be wider")]
    fn rejects_a_window_wider_than_the_half() {
        let _ = Table::identity_and_offset::<Sv39>(
            HIGH_BASE,
            (ROOT_ENTRIES_PER_HALF + 1) * GIGAPAGE,
            BOOT,
        );
    }

    #[test]
    fn root_page_entry_encodes_ppn_and_flags() {
        let mut table = Table::new();
        let pa = PhysicalAddr::new(2 * GIGAPAGE);
        table.map_root_page::<Sv39>(VirtualAddr::new(3 * GIGAPAGE), pa, BOOT);

        let entry = table.entries[3];
        assert_eq!(entry.target(), pa, "entry must carry the mapped frame");
        assert!(entry.flags().contains(PteFlags::VALID), "VALID is applied automatically");
        assert!(entry.is_leaf(), "a root page is a leaf");
    }

    #[test]
    #[should_panic(expected = "aligned to a whole root slot")]
    fn rejects_a_misaligned_root_page() {
        let mut table = Table::new();
        table.map_root_page::<Sv39>(VirtualAddr::new(0x1000), PhysicalAddr::new(0), BOOT);
    }

    #[test]
    #[should_panic(expected = "aligned to a whole root slot")]
    fn root_page_alignment_is_the_schemes_to_decide() {
        let mut table = Table::new();
        table.map_root_page::<Sv48>(VirtualAddr::new(GIGAPAGE), PhysicalAddr::new(0), BOOT);
    }

    #[test]
    #[should_panic(expected = "high canonical half")]
    fn rejects_an_offset_outside_the_high_half() {
        let _ = Table::identity_and_offset::<Sv39>(GIGAPAGE, SPAN, BOOT);
    }

    #[test]
    #[should_panic(expected = "root-page boundary")]
    fn rejects_an_unaligned_offset() {
        let _ = Table::identity_and_offset::<Sv39>(HIGH_BASE + PAGE_SIZE, SPAN, BOOT);
    }
}
