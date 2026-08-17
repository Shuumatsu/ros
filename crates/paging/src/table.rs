//! The page-table type: one page of hardware-defined entries.
//!
//! `Table` is pure data, plus only those operations that touch **this table's
//! own entries** — no frame allocation, no way to reach another frame. That is
//! what keeps the type usable in a `const` initializer and in code running
//! before any allocator exists.
//!
//! The type is scheme-independent — every RV64 scheme fills one page with 512 entries —
//! but the two builders below install *root* leaves, and what a root leaf covers is a
//! scheme's business. So they take one as a parameter while the type does not.
//!
//! Anything that descends into child tables needs the caller's frame source and
//! addressing policy, so it lives in [`crate::mapper`] instead.

use core::mem::{align_of, size_of};

use crate::addr::{PhysicalAddr, VirtualAddr};
use crate::geometry::{ENTRIES_PER_PAGE, PAGE_SIZE, ROOT_ENTRIES_PER_HALF};
use crate::pte::{Entry, PteFlags};
use crate::scheme::Scheme;

/// A single page table: 512 entries filling exactly one 4 KiB frame.
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

    /// Install a leaf covering one whole root slot directly in this root table:
    /// `S::ROOT_PAGE` bytes, which is 1 GiB under Sv39.
    ///
    /// A root-level leaf has no intermediate tables beneath it, so this
    /// allocates nothing and needs no way to reach other frames. That makes it
    /// the one mapping operation usable in a `const` initializer and in early
    /// boot code — which is exactly what the boot page table is built from.
    ///
    /// For anything smaller, or for a tree that already exists, use
    /// [`Mapper`](crate::Mapper).
    ///
    /// # Panics
    ///
    /// If `vaddr` or `paddr` is not aligned to a root page, or `flags` is not a
    /// legal leaf. In a `const` context these are compile-time errors.
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
        self.entries[vaddr.vpn(S::ROOT_LEVEL)] = Entry::leaf(paddr, flags);
    }

    /// The table a higher-half kernel enters paging on: the low canonical half
    /// identity mapped (`VA == PA`) with a root page in every slot, and the first
    /// `offset_span` bytes of physical memory mirrored at `va_offset`
    /// (`VA == PA + va_offset`).
    ///
    /// Both halves are needed, and for different instructions. The low one keeps
    /// the fetch after the `satp` write working, because the program counter is
    /// still a physical address at that point; the high one is where the very next
    /// jump goes. Filling every low slot rather than just the kernel's own costs
    /// nothing — a root table is one page either way — and it means the device
    /// tree, wherever the loader put it, is reachable before translation is on.
    ///
    /// The high half is **not** filled the same way. `offset_span` is the caller's
    /// direct-map window, and the slots above it are left invalid on purpose: a
    /// kernel that hands out high addresses of its own above that window must not
    /// find them already mapped here, to physical memory that need not exist.
    ///
    /// `flags` applies verbatim to all of it, so this grants one blanket
    /// permission over all of memory and is meant to be replaced by a table with
    /// per-section rights as soon as there is an allocator to build one.
    ///
    /// # Panics
    ///
    /// If `va_offset` is not the base of the high canonical half, or `offset_span`
    /// is not a non-zero number of whole root pages that fits in that half. In a
    /// `const` context these are compile-time errors.
    pub const fn identity_and_offset<S: Scheme>(
        va_offset: usize,
        offset_span: usize,
        flags: PteFlags,
    ) -> Self {
        let root_page = S::ROOT_PAGE;
        assert!(va_offset % root_page == 0, "the high half must begin on a root-page boundary");
        assert!(
            VirtualAddr::new(va_offset).vpn(S::ROOT_LEVEL) == ROOT_ENTRIES_PER_HALF,
            "va_offset must be the base of the high canonical half"
        );
        assert!(
            offset_span % root_page == 0,
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
                table.map_root_page::<S>(VirtualAddr::new(va_offset + index * root_page), pa, flags);
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
    use crate::scheme::{Sv39, Sv48};

    /// Bottom of the Sv39 high half, where a higher-half kernel lives.
    const HIGH_BASE: usize = 0xffff_ffc0_0000_0000;
    /// A direct-map window narrower than the half holding it, as a real kernel's is:
    /// half of it, leaving the other half for addresses the kernel chooses.
    const SPAN: usize = (ROOT_ENTRIES_PER_HALF / 2) * GIGAPAGE;
    /// The permissions an early boot mapping needs: all access, and A/D
    /// pre-set so the hardware walker never has to write to the table.
    const BOOT: PteFlags =
        PteFlags::READ_WRITE_EXECUTE.union(PteFlags::ACCESS).union(PteFlags::DIRTY);

    /// The kernel's boot table, forced through const evaluation: if
    /// [`Table::identity_and_offset`] were not truly const-usable, this would not
    /// compile, and the kernel could not hold it as a `static`.
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
        // The high half starts at root index 256 — derived, not assumed.
        let high_index = VirtualAddr::new(HIGH_BASE).vpn(Sv39::ROOT_LEVEL);
        assert_eq!(high_index, ROOT_ENTRIES_PER_HALF, "high half begins at root entry 256");
        let window_slots = SPAN / GIGAPAGE;

        for i in 0..ROOT_ENTRIES_PER_HALF {
            let expected = PhysicalAddr::new(i * GIGAPAGE);

            let identity = EARLY.entries[i];
            assert!(identity.is_leaf(), "identity entry {i} must be a leaf, not a branch");
            assert_eq!(identity.target(), expected, "identity entry {i} targets the wrong frame");

            let high = EARLY.entries[high_index + i];
            if i >= window_slots {
                // Above the direct-map window: this is where the kernel hands out
                // addresses of its own, and a mapping here would collide with them.
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

    /// A full-width window is still legal — the split is the caller's policy, not this
    /// function's — and it is the boundary case of the slot arithmetic.
    #[test]
    fn a_full_width_offset_half_fills_the_table() {
        let table =
            Table::identity_and_offset::<Sv39>(HIGH_BASE, ROOT_ENTRIES_PER_HALF * GIGAPAGE, BOOT);
        assert!(
            table.entries.iter().all(|entry| entry.is_valid()),
            "a window as wide as the half must leave no slot invalid"
        );
    }

    /// The same builder under a deeper scheme: a root table is 512 entries in every
    /// scheme, but each Sv48 slot spans 512 GiB rather than 1 GiB, so the window and the
    /// high-half base are measured in those.
    #[test]
    fn the_builder_follows_the_scheme_it_is_given() {
        const SV48_HIGH_BASE: usize = 0xffff_8000_0000_0000;
        let span = 4 * Sv48::ROOT_PAGE;
        let table = Table::identity_and_offset::<Sv48>(SV48_HIGH_BASE, span, BOOT);

        let high_index = VirtualAddr::new(SV48_HIGH_BASE).vpn(Sv48::ROOT_LEVEL);
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

    /// Slots past the end of the half belong to the *low* half, so an over-wide window
    /// would silently overwrite the identity map the `satp` write depends on.
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

    /// An address a gigapage apart is aligned under Sv39 and misaligned under Sv48, so the
    /// same call is accepted or rejected according to the scheme.
    #[test]
    #[should_panic(expected = "aligned to a whole root slot")]
    fn root_page_alignment_is_the_schemes_to_decide() {
        let mut table = Table::new();
        table.map_root_page::<Sv48>(VirtualAddr::new(GIGAPAGE), PhysicalAddr::new(0), BOOT);
    }

    /// An offset that is not the base of the high half would put the kernel's own
    /// mappings at root slots the low half already owns, silently overwriting the
    /// identity map the `satp` write depends on.
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
