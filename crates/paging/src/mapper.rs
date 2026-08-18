//! Page-table walks: mapping, translation and teardown.
//!
//! Everything here descends *through* tables, so everything here needs the three things
//! the caller owns: how deep the tree is ([`Scheme`]), where new intermediate tables come
//! from ([`FrameSource`]) and how to reach a frame from its physical address
//! ([`PhysAccess`]). [`Mapper`] binds a root table to one choice of each, which keeps that
//! choice fixed for the whole tree and keeps it off [`Table`] — a type that must stay
//! exactly one page of hardware-defined entries.
//!
//! The walk itself is written once for every scheme. Sv39, Sv48 and Sv57 differ only in
//! where it starts, so `S::ROOT_LEVEL` is the whole of the difference in this file.

use core::marker::PhantomData;

use crate::access::PhysAccess;
use crate::addr::{MemoryAddr, PhysicalAddr, VirtualAddr};
use crate::frames::FrameSource;
use crate::geometry::{ENTRIES_PER_PAGE, PAGE_OFFSET_BITS, VPN_BITS, page_size_at};
use crate::pte::{Entry, PteFlags};
use crate::scheme::{Scheme, vpn};
use crate::table::Table;
use crate::utils::mask;

/// Why a mapping could not be installed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MapError {
    /// `level` is not one of the scheme's page-table levels.
    #[error("page-table level {level} is out of range for a {levels}-level scheme")]
    InvalidLevel {
        /// The rejected level.
        level: usize,
        /// Levels the scheme in use actually walks.
        levels: usize,
    },
    /// The virtual address is not aligned to the page size it would map.
    #[error("virtual address {vaddr:#x} is not aligned to its {page_size:#x}-byte page")]
    UnalignedVirtual {
        /// The rejected virtual address.
        vaddr: VirtualAddr,
        /// Page size implied by the requested level.
        page_size: usize,
    },
    /// The physical address is not aligned to the page size it would map.
    #[error("physical address {paddr:#x} is not aligned to its {page_size:#x}-byte page")]
    UnalignedPhysical {
        /// The rejected physical address.
        paddr: PhysicalAddr,
        /// Page size implied by the requested level.
        page_size: usize,
    },
    /// The flags name none of R/W/X, which encodes a branch rather than a page.
    #[error("a mapping needs at least one of R/W/X")]
    NotALeaf,
    /// Write-without-read is architecturally reserved.
    #[error("write-without-read is a reserved PTE encoding")]
    WriteWithoutRead,
    /// The frame source is exhausted, so no intermediate table could be made.
    #[error("no frame available for an intermediate page table")]
    OutOfFrames,
    /// A superpage already covers this address, so the walk cannot descend.
    #[error("an existing superpage at level {level} already covers this address")]
    SuperpageInPath {
        /// Level at which the blocking leaf was found.
        level: usize,
    },
}

/// A leaf that [`Mapper::unmap`] removed.
///
/// The frame is reported as its own base rather than as the translation of the
/// address that was passed in: an unmapped page is only useful as a whole, so the
/// low bits of the caller's address have no meaning left.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Unmapped {
    /// Base of the frame the leaf pointed at.
    pub frame: PhysicalAddr,
    /// Level the leaf was installed at: 0 = 4 KiB, 1 = 2 MiB, 2 = 1 GiB.
    pub level: usize,
}

impl Unmapped {
    /// Bytes the leaf covered.
    pub fn len(&self) -> usize { page_size_at(self.level) }
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

/// Reject a mapping request that the hardware could not represent.
fn validate<S: Scheme>(
    vaddr: VirtualAddr,
    paddr: PhysicalAddr,
    level: usize,
    flags: PteFlags,
) -> Result<(), MapError> {
    // Checked first: `page_size_at` is only defined for real levels.
    if level >= S::LEVELS {
        return Err(MapError::InvalidLevel { level, levels: S::LEVELS });
    }
    if !flags.is_leaf() {
        return Err(MapError::NotALeaf);
    }
    if !flags.is_legal_leaf() {
        return Err(MapError::WriteWithoutRead);
    }
    let page_size = page_size_at(level);
    if !vaddr.is_aligned(page_size) {
        return Err(MapError::UnalignedVirtual { vaddr, page_size });
    }
    if !paddr.is_aligned(page_size) {
        return Err(MapError::UnalignedPhysical { paddr, page_size });
    }
    Ok(())
}

/// A page-table tree plus the policy needed to walk it.
///
/// `S` is a compile-time marker, never a value: which scheme a tree is built for is fixed
/// when the root frame is allocated, so carrying it in a field would be storing a constant.
pub struct Mapper<'a, S, F, A> {
    root: &'a mut Table,
    frames: F,
    access: A,
    scheme: PhantomData<S>,
}

impl<'a, S: Scheme, F: FrameSource, A: PhysAccess> Mapper<'a, S, F, A> {
    /// Bind `root` to a scheme, a frame source and an addressing strategy.
    pub fn new(root: &'a mut Table, frames: F, access: A) -> Self {
        Self { root, frames, access, scheme: PhantomData }
    }

    /// Borrow the frame source, for callers that share one allocator.
    pub fn frames_mut(&mut self) -> &mut F { &mut self.frames }

    /// Map a single 4 KiB page `vaddr -> paddr`.
    pub fn map(
        &mut self,
        vaddr: VirtualAddr,
        paddr: PhysicalAddr,
        flags: PteFlags,
    ) -> Result<(), MapError> {
        self.map_at_level(vaddr, paddr, 0, flags)
    }

    /// Map one page of the size `level` selects: 4 KiB (0), 2 MiB (1), 1 GiB (2).
    ///
    /// Intermediate tables are created for every level *above* `level`, so a
    /// root-level mapping never touches the frame source. `VALID` is applied
    /// automatically; `flags` must name at least one of R/W/X.
    ///
    /// On [`MapError::OutOfFrames`] any intermediate tables already created are
    /// left in place. They are empty and will be reused by the next mapping
    /// through the same region — nothing is corrupted, but nothing is unwound.
    pub fn map_at_level(
        &mut self,
        vaddr: VirtualAddr,
        paddr: PhysicalAddr,
        level: usize,
        flags: PteFlags,
    ) -> Result<(), MapError> {
        validate::<S>(vaddr, paddr, level, flags)?;

        let mut table: *mut Table = self.root;
        let mut current = S::ROOT_LEVEL;
        while current > level {
            let index = vpn::<S>(vaddr, current);
            // Read the entry out by value: holding a `&mut` into the table
            // across the frame-source call would borrow `self` twice.
            // SAFETY: `table` is the root or a child reached through a valid
            // branch entry, and `access` maps such frames to live pointers.
            let existing = unsafe { (*table).entries[index] };

            let child = if !existing.is_valid() {
                let frame = self.frames.alloc_zeroed().ok_or(MapError::OutOfFrames)?;
                // SAFETY: same frame as above; the slot is ours to overwrite.
                unsafe { (*table).entries[index] = Entry::branch(frame) };
                frame
            } else if existing.is_leaf() {
                return Err(MapError::SuperpageInPath { level: current });
            } else {
                existing.target()
            };

            table = self.access.ptr::<Table>(child);
            current -= 1;
        }

        // SAFETY: the walk stopped at `level`, so `table` is the table that
        // owns the entry for `vaddr` at that level.
        unsafe { (*table).entries[vpn::<S>(vaddr, level)] = Entry::leaf(paddr, flags) };
        Ok(())
    }

    /// Find the leaf entry that maps `vaddr`, with the level it was found at.
    ///
    /// Returns `None` if the walk hits an invalid entry, or a branch where a leaf
    /// must be.
    ///
    /// Exposed so a caller can audit *permissions*, not just addresses:
    /// [`translate`](Self::translate) discards the flags, and checking that a
    /// freshly built kernel table is actually W^X needs them. The level comes back
    /// too, because a leaf at level 1 or 2 is a superpage and "which page size did
    /// this region really get" is part of the audit.
    pub fn entry_of(&self, vaddr: VirtualAddr) -> Option<(Entry, usize)> {
        let mut table: *const Table = self.root;
        for level in (0..S::LEVELS).rev() {
            // SAFETY: `table` is the root or a child from a valid branch entry.
            let entry = unsafe { (*table).entries[vpn::<S>(vaddr, level)] };
            if !entry.is_valid() {
                return None;
            }
            if entry.is_leaf() {
                return Some((entry, level));
            }
            table = self.access.ptr::<Table>(entry.target());
        }
        // Fell off the bottom without a leaf: a branch at level 0 is malformed.
        None
    }

    /// Translate a virtual address, honouring leaf mappings at any level.
    ///
    /// Returns `None` if the walk hits an invalid entry, or a branch where a
    /// leaf must be.
    pub fn translate(&self, vaddr: VirtualAddr) -> Option<PhysicalAddr> {
        self.entry_of(vaddr).map(|(entry, level)| leaf_to_phys(entry, vaddr, level))
    }

    /// Remove the leaf that maps `vaddr`, whatever page size it was installed at,
    /// and report what was there.
    ///
    /// The frame is **not** released: this crate does not own leaf-mapped memory,
    /// so whether it goes back to an allocator is the caller's decision —
    /// [`Unmapped::frame`] and [`Unmapped::level`] are what it needs to make it.
    ///
    /// Intermediate tables are left in place. They are shared with every
    /// neighbouring mapping in the same region, so freeing one because *this* leaf
    /// went away would need a per-table occupancy count that only pays off when a
    /// whole tree is being torn down — which is [`free_subtables`](Self::free_subtables).
    ///
    /// # The TLB is not flushed
    ///
    /// It cannot be: `sfence.vma` is an instruction, and this crate is data
    /// structures. Until the caller executes one, this hart may keep translating
    /// `vaddr` through the entry that was just cleared. Unmapping a range means one
    /// flush after the last page, not one per page, so the choice has to belong to
    /// the caller.
    ///
    /// The walk is [`entry_of`](Self::entry_of)'s, repeated rather than shared for
    /// the same reason `get` and `get_mut` are two functions: that one hands out an
    /// entry read through a `&self`, and this one writes through the table holding it.
    pub fn unmap(&mut self, vaddr: VirtualAddr) -> Option<Unmapped> {
        let mut table: *mut Table = self.root;
        for level in (0..S::LEVELS).rev() {
            let index = vpn::<S>(vaddr, level);
            // SAFETY: `table` is the root or a child from a valid branch entry, and
            // `access` maps such frames to live pointers.
            let entry = unsafe { (*table).entries[index] };
            if !entry.is_valid() {
                return None;
            }
            if entry.is_leaf() {
                // SAFETY: same table; the slot is the one that maps `vaddr`.
                unsafe { (*table).entries[index] = Entry::empty() };
                return Some(Unmapped { frame: entry.target(), level });
            }
            table = self.access.ptr::<Table>(entry.target());
        }
        // Fell off the bottom without a leaf: a branch at level 0 is malformed.
        None
    }

    /// Map every page of the size `level` selects that overlaps `[start, end)`,
    /// taking each physical address from `translate`.
    ///
    /// The range is rounded **outward** to whole pages, so a partial page at
    /// either end is still fully mapped. At level 0 that only ever adds bytes
    /// inside the caller's own final page; at a superpage level it can pull in a
    /// substantial neighbourhood, because "cover this range with 2 MiB pages"
    /// cannot mean anything else. A caller needing exactness should align its
    /// range and assert it rather than rely on the rounding.
    pub fn map_range_at_level<T>(
        &mut self,
        start: VirtualAddr,
        end: VirtualAddr,
        level: usize,
        translate: T,
        flags: PteFlags,
    ) -> Result<(), MapError>
    where
        T: Fn(VirtualAddr) -> PhysicalAddr,
    {
        // Checked before `page_size_at`, which is only defined for real levels.
        if level >= S::LEVELS {
            return Err(MapError::InvalidLevel { level, levels: S::LEVELS });
        }
        let page = page_size_at(level);
        let mut va = start.align_down(page);
        let end = end.align_up(page);
        while va < end {
            self.map_at_level(va, translate(va), level, flags)?;
            va = va.add(page);
        }
        Ok(())
    }

    /// Map every 4 KiB page overlapping `[start, end)`, taking each physical
    /// address from `translate`. `end` is rounded up so a partial final page is
    /// still fully mapped.
    pub fn map_range<T>(
        &mut self,
        start: VirtualAddr,
        end: VirtualAddr,
        translate: T,
        flags: PteFlags,
    ) -> Result<(), MapError>
    where
        T: Fn(VirtualAddr) -> PhysicalAddr,
    {
        self.map_range_at_level(start, end, 0, translate, flags)
    }

    /// Identity-map `[start, end)` (each virtual page to the equal physical page).
    pub fn id_map_range(
        &mut self,
        start: VirtualAddr,
        end: VirtualAddr,
        flags: PteFlags,
    ) -> Result<(), MapError> {
        self.map_range(start, end, |v| PhysicalAddr::new(v.bits()), flags)
    }

    /// Return every intermediate table beneath the root to the frame source.
    ///
    /// Leaf-mapped frames are left untouched (they are not owned here) and the
    /// root itself is not freed. Freed branch entries are cleared, so the tree
    /// is left consistent.
    ///
    /// # Safety
    ///
    /// Every branch under the root must point to a frame from this mapper's
    /// [`FrameSource`], and none of them may still be in use by hardware — the
    /// caller must have removed this tree from `satp` and flushed the TLB.
    pub unsafe fn free_subtables(&mut self) {
        let root: *mut Table = self.root;
        // SAFETY: forwarded from this function's contract.
        unsafe { self.free_below(root, S::ROOT_LEVEL) };
    }

    /// Recursively free the branches under `table`, which sits at `level`.
    ///
    /// # Safety
    /// See [`free_subtables`](Self::free_subtables); `table` must be live and
    /// at `level`.
    unsafe fn free_below(&mut self, table: *mut Table, level: usize) {
        // Entries at level 0 are leaves; there is nothing below them to free.
        if level == 0 {
            return;
        }
        for index in 0..ENTRIES_PER_PAGE {
            // SAFETY: `table` is live for `ENTRIES_PER_PAGE` entries.
            let entry = unsafe { (*table).entries[index] };
            if !entry.is_branch() {
                continue;
            }
            let child = self.access.ptr::<Table>(entry.target());
            // SAFETY: a branch points at a frame this source handed out; free
            // its descendants first (post-order), then the frame itself, then
            // clear the slot so the tree never references freed memory.
            unsafe {
                self.free_below(child, level - 1);
                self.frames.free(entry.target());
                (*table).entries[index] = Entry::empty();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::Identity;
    use crate::geometry::PAGE_SIZE;
    use crate::scheme::{Sv39, Sv48};

    /// The scheme these tests walk. Named once: what is being exercised is the walk,
    /// not the level count, and `walks_the_scheme_it_is_given` covers the other case.
    type Sv39Mapper<'a, F> = Mapper<'a, Sv39, F, Identity>;

    /// A frame source backed by boxed tables.
    ///
    /// A boxed `Table` is page-aligned (the type demands it) and its heap block
    /// does not move when the `Vec` grows, so the addresses handed out stay
    /// valid. That is what lets host tests use [`Identity`] and treat a host
    /// pointer as a physical address.
    #[derive(Default)]
    struct Arena {
        tables: Vec<Box<Table>>,
        freed: Vec<usize>,
    }

    // SAFETY: every frame is a freshly boxed `Table::new()` — page-aligned,
    // zeroed (all entries invalid), and owned solely by this arena.
    unsafe impl FrameSource for Arena {
        fn alloc_zeroed(&mut self) -> Option<PhysicalAddr> {
            let mut table = Box::new(Table::new());
            let pa = PhysicalAddr::new(table.as_mut() as *mut Table as usize);
            self.tables.push(table);
            Some(pa)
        }
        unsafe fn free(&mut self, frame: PhysicalAddr) {
            // Record rather than deallocate: the arena owns the boxes until it
            // drops, which keeps freed frames safe to inspect in assertions.
            self.freed.push(frame.bits());
        }
    }

    /// A frame source that never yields, for proving a path allocates nothing.
    struct Barren;

    // SAFETY: never hands out a frame, so the contract holds vacuously.
    unsafe impl FrameSource for Barren {
        fn alloc_zeroed(&mut self) -> Option<PhysicalAddr> { None }
        unsafe fn free(&mut self, _frame: PhysicalAddr) {}
    }

    const RWX: PteFlags = PteFlags::READ_WRITE_EXECUTE;

    /// `map_range` is the level-0 case of `map_range_at_level`, so the two must agree
    /// exactly, rounding included.
    #[test]
    fn map_range_is_the_level_zero_case_of_map_range_at_level() {
        let base = VirtualAddr::new(7 << 21);
        let span = 3 * PAGE_SIZE + 0x40; // deliberately not a whole number of pages
        let phys = |v: VirtualAddr| PhysicalAddr::new(v.bits() - base.bits() + 0x8000_0000);

        let mut lhs = Table::new();
        let mut generic = Sv39Mapper::new(&mut lhs, Arena::default(), Identity);
        generic
            .map_range_at_level(base, base.add(span), 0, phys, PteFlags::READ_WRITE)
            .expect("explicit level-0 range must map");

        let mut rhs = Table::new();
        let mut shorthand = Sv39Mapper::new(&mut rhs, Arena::default(), Identity);
        shorthand
            .map_range(base, base.add(span), phys, PteFlags::READ_WRITE)
            .expect("shorthand range must map");

        // Four pages: the partial tail is rounded outward and fully mapped.
        for index in 0..4 {
            let va = base.add(index * PAGE_SIZE);
            assert_eq!(
                generic.translate(va),
                Some(phys(va)),
                "page {index} missing from the explicit-level mapping"
            );
            assert_eq!(
                shorthand.translate(va),
                generic.translate(va),
                "page {index} differs between map_range and map_range_at_level"
            );
        }
        let past = base.add(4 * PAGE_SIZE);
        assert_eq!(generic.translate(past), None, "rounding must not overshoot a whole page");
        assert_eq!(shorthand.translate(past), None, "shorthand must not overshoot either");
    }

    #[test]
    fn map_range_at_level_builds_superpages_and_rejects_a_bad_level() {
        let mut root = Table::new();
        let mut mapper = Sv39Mapper::new(&mut root, Arena::default(), Identity);
        let base = VirtualAddr::new(4 << 30);
        let superpage = page_size_at(1);

        mapper
            .map_range_at_level(
                base,
                base.add(2 * superpage),
                1,
                |v| PhysicalAddr::new(v.bits() - base.bits()),
                PteFlags::READ_WRITE,
            )
            .expect("2 MiB range must map");

        for index in 0..2 {
            let va = base.add(index * superpage);
            let (_, level) = mapper.entry_of(va).expect("superpage must be mapped");
            assert_eq!(level, 1, "range must be built from level-1 leaves, not 4 KiB ones");
        }

        assert_eq!(
            mapper.map_range_at_level(
                base,
                base.add(PAGE_SIZE),
                Sv39::LEVELS,
                |_| PhysicalAddr::new(0),
                RWX
            ),
            Err(MapError::InvalidLevel { level: Sv39::LEVELS, levels: Sv39::LEVELS }),
            "an out-of-range level must be rejected, not used to index a page size"
        );
    }

    /// `entry_of` is what lets a caller audit permissions rather than trust them,
    /// so it must report the flags and the level a mapping actually landed at —
    /// not the ones that were requested.
    #[test]
    fn entry_of_reports_the_flags_and_level_a_mapping_landed_at() {
        let mut root = Table::new();
        let mut mapper = Sv39Mapper::new(&mut root, Arena::default(), Identity);

        let text = VirtualAddr::new(4 << 21);
        let data = VirtualAddr::new(5 << 21);
        let giga = VirtualAddr::new(3 << 30);
        mapper
            .map(text, PhysicalAddr::new(0x8000_0000), PteFlags::READ_EXECUTE)
            .expect("R+X page must map");
        mapper
            .map_at_level(data, PhysicalAddr::new(0x8020_0000), 1, PteFlags::READ_WRITE)
            .expect("R+W superpage must map");
        mapper
            .map_at_level(giga, PhysicalAddr::new(0), Sv39::ROOT_LEVEL, PteFlags::READ)
            .expect("R gigapage must map");

        let (entry, level) = mapper.entry_of(text).expect("mapped page must have an entry");
        assert_eq!(level, 0, "a 4 KiB mapping must be reported at level 0");
        assert_eq!(
            entry.flags(),
            PteFlags::READ_EXECUTE | PteFlags::VALID,
            "R+X flags must survive the round trip"
        );
        assert!(!entry.flags().contains(PteFlags::WRITE), "executable page must not be writable");

        let (entry, level) = mapper.entry_of(data).expect("mapped superpage must have an entry");
        assert_eq!(level, 1, "a 2 MiB mapping must be reported at level 1");
        assert!(!entry.flags().contains(PteFlags::EXECUTE), "writable page must not be executable");

        let (_, level) = mapper.entry_of(giga).expect("mapped gigapage must have an entry");
        assert_eq!(level, Sv39::ROOT_LEVEL, "a 1 GiB mapping must be reported at the root level");

        // An unmapped address — a guard page, say — must be reported as a hole,
        // which is how a self-check proves a gap was left deliberately.
        let hole = VirtualAddr::new(6 << 21);
        assert_eq!(mapper.entry_of(hole), None, "unmapped address must have no entry");
        assert_eq!(mapper.translate(hole), None, "translate must agree with entry_of");
    }

    #[test]
    fn maps_and_translates_a_four_kib_page() {
        let mut root = Table::new();
        let mut mapper = Sv39Mapper::new(&mut root, Arena::default(), Identity);
        // A distinct index at every level: vpn2=1, vpn1=2, vpn0=3.
        let va = VirtualAddr::new((1 << 30) | (2 << 21) | (3 << 12));
        let pa = PhysicalAddr::new(0x8020_1000);

        mapper.map(va, pa, PteFlags::READ_WRITE).expect("4 KiB mapping must succeed");
        assert_eq!(mapper.translate(va), Some(pa), "exact page translates back");

        let va_off = VirtualAddr::new(va.bits() + 0x123);
        let pa_off = PhysicalAddr::new(pa.bits() + 0x123);
        assert_eq!(mapper.translate(va_off), Some(pa_off), "in-page offset preserved");
    }

    #[test]
    fn maps_and_translates_superpages() {
        for (level, size) in [(1usize, 2 * 1024 * 1024usize), (2, 1024 * 1024 * 1024)] {
            let mut root = Table::new();
            let mut mapper = Sv39Mapper::new(&mut root, Arena::default(), Identity);
            let va = VirtualAddr::new(4 * size);
            let pa = PhysicalAddr::new(6 * size);

            mapper
                .map_at_level(va, pa, level, RWX)
                .unwrap_or_else(|e| panic!("level-{level} mapping must succeed: {e}"));
            assert_eq!(mapper.translate(va), Some(pa), "level-{level} base translates");

            // An offset anywhere inside the superpage resolves through it.
            let inside = size - 1;
            assert_eq!(
                mapper.translate(VirtualAddr::new(va.bits() + inside)),
                Some(PhysicalAddr::new(pa.bits() + inside)),
                "offset inside a level-{level} superpage is preserved",
            );
        }
    }

    #[test]
    fn a_gigapage_needs_no_frames() {
        // The early boot table depends on this: a root-level leaf must never
        // reach for the frame source, because none exists yet.
        let mut root = Table::new();
        let mut mapper = Sv39Mapper::new(&mut root, Barren, Identity);
        let va = VirtualAddr::new(2 << 30);
        let pa = PhysicalAddr::new(2 << 30);

        mapper
            .map_at_level(va, pa, Sv39::ROOT_LEVEL, RWX)
            .expect("a root-level gigapage must not allocate");
        assert_eq!(mapper.translate(va), Some(pa), "gigapage translates");
    }

    #[test]
    fn reports_out_of_frames_below_the_root() {
        let mut root = Table::new();
        let mut mapper = Sv39Mapper::new(&mut root, Barren, Identity);
        let error = mapper
            .map(VirtualAddr::new(0x4000), PhysicalAddr::new(0x4000), RWX)
            .expect_err("a 4 KiB mapping needs intermediate tables");
        assert_eq!(error, MapError::OutOfFrames, "wrong exhaustion diagnostic");
    }

    #[test]
    fn rejects_a_superpage_in_the_walk_path() {
        let mut root = Table::new();
        let mut mapper = Sv39Mapper::new(&mut root, Arena::default(), Identity);
        let giga = 1 << 30;
        mapper
            .map_at_level(VirtualAddr::new(giga), PhysicalAddr::new(giga), Sv39::ROOT_LEVEL, RWX)
            .expect("gigapage must map");

        // A 4 KiB mapping inside that gigapage would have to descend through it.
        let error = mapper
            .map(VirtualAddr::new(giga + 0x1000), PhysicalAddr::new(0x1000), RWX)
            .expect_err("cannot descend through a superpage");
        assert_eq!(
            error,
            MapError::SuperpageInPath { level: Sv39::ROOT_LEVEL },
            "wrong blocked-walk diagnostic",
        );
    }

    #[test]
    fn rejects_misaligned_and_illegal_requests() {
        let mut root = Table::new();
        let mut mapper = Sv39Mapper::new(&mut root, Arena::default(), Identity);
        let two_mib = 2 * 1024 * 1024;

        assert_eq!(
            mapper.map_at_level(VirtualAddr::new(0x1000), PhysicalAddr::new(0), 1, RWX),
            Err(MapError::UnalignedVirtual { vaddr: VirtualAddr::new(0x1000), page_size: two_mib }),
            "a 2 MiB mapping needs a 2 MiB-aligned VA",
        );
        assert_eq!(
            mapper.map_at_level(VirtualAddr::new(0), PhysicalAddr::new(0x1000), 1, RWX),
            Err(MapError::UnalignedPhysical {
                paddr: PhysicalAddr::new(0x1000),
                page_size: two_mib
            }),
            "a 2 MiB mapping needs a 2 MiB-aligned PA",
        );
        assert_eq!(
            mapper.map(VirtualAddr::new(0), PhysicalAddr::new(0), PteFlags::VALID),
            Err(MapError::NotALeaf),
            "no R/W/X is a branch, not a page",
        );
        assert_eq!(
            mapper.map(VirtualAddr::new(0), PhysicalAddr::new(0), PteFlags::WRITE),
            Err(MapError::WriteWithoutRead),
            "write-only is reserved",
        );
        assert_eq!(
            mapper.map_at_level(VirtualAddr::new(0), PhysicalAddr::new(0), Sv39::LEVELS, RWX),
            Err(MapError::InvalidLevel { level: Sv39::LEVELS, levels: Sv39::LEVELS }),
            "level must be a real Sv39 level",
        );
    }

    #[test]
    fn unmapped_translates_to_none() {
        let mut root = Table::new();
        let mut mapper = Sv39Mapper::new(&mut root, Arena::default(), Identity);
        mapper
            .map(
                VirtualAddr::new(0x8000_0000),
                PhysicalAddr::new(0x8000_0000),
                PteFlags::READ_WRITE,
            )
            .expect("mapping must succeed");

        assert_eq!(mapper.translate(VirtualAddr::new(0x9000_0000)), None, "unmapped VA is None");
    }

    #[test]
    fn map_range_covers_a_partial_final_page() {
        let mut root = Table::new();
        let mut mapper = Sv39Mapper::new(&mut root, Arena::default(), Identity);
        // 0x3001 lands in the page based at 0x3000, so three pages must be
        // mapped; rounding the end down would have dropped it.
        mapper
            .id_map_range(VirtualAddr::new(0x1000), VirtualAddr::new(0x3001), PteFlags::READ_WRITE)
            .expect("range must map");

        for page in [0x1000usize, 0x2000, 0x3000] {
            assert_eq!(
                mapper.translate(VirtualAddr::new(page)),
                Some(PhysicalAddr::new(page)),
                "page {page:#x} in range must be mapped",
            );
        }
        assert_eq!(mapper.translate(VirtualAddr::new(0x4000)), None, "page past the range");
        assert_eq!(mapper.translate(VirtualAddr::new(0)), None, "page before the range");
    }

    #[test]
    fn remapping_overwrites_the_leaf() {
        let mut root = Table::new();
        let mut mapper = Sv39Mapper::new(&mut root, Arena::default(), Identity);
        let va = VirtualAddr::new(0x4000_0000);
        mapper.map(va, PhysicalAddr::new(0x1000), PteFlags::READ).expect("first map");
        mapper.map(va, PhysicalAddr::new(0x2000), PteFlags::READ_WRITE).expect("second map");

        assert_eq!(mapper.translate(va), Some(PhysicalAddr::new(0x2000)), "second map wins");
    }

    #[test]
    fn free_subtables_returns_every_intermediate_frame() {
        let mut root = Table::new();
        let mut mapper = Sv39Mapper::new(&mut root, Arena::default(), Identity);
        // One 4 KiB mapping builds exactly two intermediate tables (levels 1, 0).
        mapper
            .map(VirtualAddr::new(0x4000_0000), PhysicalAddr::new(0x1000), PteFlags::READ_WRITE)
            .expect("mapping must succeed");
        let allocated = mapper.frames_mut().tables.len();
        assert_eq!(allocated, 2, "a 4 KiB mapping needs a level-1 and a level-0 table");

        // SAFETY: this tree is not installed in any satp; the arena owns the frames.
        unsafe { mapper.free_subtables() };

        assert_eq!(mapper.frames_mut().freed.len(), allocated, "every intermediate must be freed");
        assert_eq!(
            mapper.translate(VirtualAddr::new(0x4000_0000)),
            None,
            "freed branches must be cleared, not left dangling",
        );
    }

    #[test]
    fn unmap_clears_one_leaf_and_leaves_its_neighbours() {
        let mut root = Table::new();
        let mut mapper = Sv39Mapper::new(&mut root, Arena::default(), Identity);
        let base = VirtualAddr::new(0x4000_0000);
        mapper
            .id_map_range(base, base.add(3 * PAGE_SIZE), PteFlags::READ_WRITE)
            .expect("range must map");

        let removed = mapper.unmap(base.add(PAGE_SIZE)).expect("the middle page was mapped");
        assert_eq!(
            removed,
            Unmapped { frame: PhysicalAddr::new(base.bits() + PAGE_SIZE), level: 0 },
            "unmap must report the frame and the size that were there",
        );
        assert_eq!(removed.len(), PAGE_SIZE, "a level-0 leaf covers one base page");
        assert_eq!(mapper.translate(base.add(PAGE_SIZE)), None, "the leaf must be gone");
        assert_eq!(
            mapper.translate(base),
            Some(PhysicalAddr::new(base.bits())),
            "the page below must survive",
        );
        assert_eq!(
            mapper.translate(base.add(2 * PAGE_SIZE)),
            Some(PhysicalAddr::new(base.bits() + 2 * PAGE_SIZE)),
            "the page above must survive",
        );
        assert!(
            mapper.frames_mut().freed.is_empty(),
            "unmap must not release anything: the leaf frame is not the mapper's, and the \
             intermediate tables are still in use by the neighbours",
        );
    }

    #[test]
    fn unmap_reports_the_level_a_superpage_was_installed_at() {
        let two_mib = page_size_at(1);
        let mut root = Table::new();
        let mut mapper = Sv39Mapper::new(&mut root, Arena::default(), Identity);
        let va = VirtualAddr::new(0x4000_0000);
        mapper.map_at_level(va, PhysicalAddr::new(0), 1, PteFlags::READ_WRITE).expect("superpage");

        // Asked from *inside* the superpage: an address in its middle must find the
        // same leaf, or a caller freeing what came back would free the wrong frame.
        let removed = mapper.unmap(va.add(PAGE_SIZE)).expect("the superpage covers this address");
        assert_eq!(
            removed,
            Unmapped { frame: PhysicalAddr::new(0), level: 1 },
            "a 2 MiB leaf must report its own base and level 1",
        );
        assert_eq!(removed.len(), two_mib, "a level-1 leaf covers 2 MiB");
        assert_eq!(mapper.translate(va), None, "the whole superpage must be gone");
    }

    #[test]
    fn unmap_of_an_unmapped_address_reports_nothing() {
        let mut root = Table::new();
        let mut mapper = Sv39Mapper::new(&mut root, Arena::default(), Identity);
        let va = VirtualAddr::new(0x4000_0000);
        mapper.map(va, PhysicalAddr::new(0x1000), PteFlags::READ_WRITE).expect("mapping");

        assert_eq!(mapper.unmap(va.add(PAGE_SIZE)), None, "never-mapped VA in a live table");
        assert_eq!(mapper.unmap(VirtualAddr::new(0x8000_0000)), None, "VA with no branch at all");
        assert_eq!(mapper.unmap(va), Some(Unmapped { frame: PhysicalAddr::new(0x1000), level: 0 }));
        assert_eq!(mapper.unmap(va), None, "a second unmap of the same page finds nothing");
    }

    #[test]
    fn unmap_keeps_the_intermediate_tables_for_the_next_mapping() {
        let mut root = Table::new();
        let mut mapper = Sv39Mapper::new(&mut root, Arena::default(), Identity);
        let va = VirtualAddr::new(0x4000_0000);
        mapper.map(va, PhysicalAddr::new(0x1000), PteFlags::READ_WRITE).expect("first mapping");
        let built = mapper.frames_mut().tables.len();
        assert_eq!(built, 2, "a 4 KiB mapping needs a level-1 and a level-0 table");

        mapper.unmap(va).expect("the leaf was there");
        mapper.map(va, PhysicalAddr::new(0x2000), PteFlags::READ_WRITE).expect("second mapping");

        // Allocating nothing the second time is the observable form of "the tables
        // are still in the tree": a torn-down branch would have to be rebuilt.
        assert_eq!(
            mapper.frames_mut().tables.len(),
            built,
            "the mapping after an unmap must reuse the tables, not build new ones",
        );
        assert_eq!(mapper.translate(va), Some(PhysicalAddr::new(0x2000)));
    }

    /// The same walk, one level deeper. Depth is observable as the number of intermediate
    /// tables a 4 KiB mapping builds — three under Sv48 where Sv39 builds two — so this
    /// fails if `S::ROOT_LEVEL` is ever bypassed for a hard-coded start.
    #[test]
    fn walks_the_scheme_it_is_given() {
        let va = VirtualAddr::new(0x4000_0000);
        let pa = PhysicalAddr::new(0x1000);

        let mut sv39_root = Table::new();
        let mut sv39 = Mapper::<Sv39, _, _>::new(&mut sv39_root, Arena::default(), Identity);
        sv39.map(va, pa, PteFlags::READ_WRITE).expect("Sv39 mapping");

        let mut sv48_root = Table::new();
        let mut sv48 = Mapper::<Sv48, _, _>::new(&mut sv48_root, Arena::default(), Identity);
        sv48.map(va, pa, PteFlags::READ_WRITE).expect("Sv48 mapping");

        assert_eq!(sv39.frames_mut().tables.len(), 2, "Sv39 descends from level 2");
        assert_eq!(sv48.frames_mut().tables.len(), 3, "Sv48 descends from level 3");
        assert_eq!(sv39.translate(va), Some(pa), "and both still translate");
        assert_eq!(sv48.translate(va), Some(pa));
    }

    /// A level the running scheme does not have is rejected against *its* count, so the
    /// same request is a root mapping under Sv48 and an error under Sv39.
    #[test]
    fn the_level_bound_is_the_schemes_own() {
        let va = VirtualAddr::new(0);
        let pa = PhysicalAddr::new(0);

        let mut sv39_root = Table::new();
        let mut sv39 = Mapper::<Sv39, _, _>::new(&mut sv39_root, Arena::default(), Identity);
        assert_eq!(
            sv39.map_at_level(va, pa, 3, RWX),
            Err(MapError::InvalidLevel { level: 3, levels: 3 }),
            "Sv39 has no level 3"
        );

        let mut sv48_root = Table::new();
        let mut sv48 = Mapper::<Sv48, _, _>::new(&mut sv48_root, Arena::default(), Identity);
        assert_eq!(sv48.map_at_level(va, pa, 3, RWX), Ok(()), "under Sv48 level 3 is the root");
    }
}
