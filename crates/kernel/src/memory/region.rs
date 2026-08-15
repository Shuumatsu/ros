//! A mapped region, and the mechanics of installing, auditing and reporting a set of
//! them.
//!
//! The mechanism, free of any particular layout — that is [`super::kernel_table`]'s, and
//! a user address space will want the same treatment over a different list. Generic over
//! the mapper's two policies so it never depends on the kernel's choice of either.
//!
//! [`Region::install`] validates before writing any PTE, rather than in a separate pass,
//! so no caller can forget: there is one way to turn a `Region` into mappings.

use paging::sv39::{FrameSource, PhysAccess, page_size_at};
use paging::{MapError, Mapper, MemoryAddr, PhysicalAddr, PteFlags, VirtualAddr};

use crate::utils::ByteSize;

/// One contiguous mapping: a virtual range, the physical range behind it, the
/// page size to build it from, and the rights it carries.
///
/// `va` and `pa` are independent, so this describes an identity mapping
/// (`va == pa`), a direct map (`va == pa + offset`) or anything else, without a
/// mode flag to branch on — [`install`](Self::install) just carries the constant
/// difference across the range.
#[derive(Clone, Copy)]
pub struct Region<'a> {
    /// What this region is, for diagnostics and the boot log. Borrowed, because a device
    /// window's name comes off the node that described it and lives in the device table
    /// rather than in the binary.
    pub name: &'a str,
    /// First virtual address.
    pub va: VirtualAddr,
    /// Physical address `va` maps to.
    pub pa: PhysicalAddr,
    /// Length in bytes. Zero means "nothing to do"; see [`is_empty`](Self::is_empty).
    pub len: usize,
    /// Page-table level to build the region from: 0 = 4 KiB, 1 = 2 MiB, 2 = 1 GiB.
    pub level: usize,
    /// Rights, minus `VALID`, which the mapper applies.
    pub flags: PteFlags,
}

impl Region<'_> {
    /// Bytes mapped by one leaf of this region.
    pub fn page_size(&self) -> usize { page_size_at(self.level) }

    /// The virtual range this region really occupies once installed: `[va, va + len)`
    /// rounded outward to whole pages.
    ///
    /// The single answer to "what address space does this take", because the rounding is
    /// [`install`](Self::install)'s — [`Mapper::map_range_at_level`] aligns both ends. An
    /// unrounded extent calls two sub-page regions sharing one page disjoint, and sharing
    /// a page means sharing a PTE, so whichever installed second would own it.
    pub fn footprint(&self) -> (VirtualAddr, VirtualAddr) {
        let page = self.page_size();
        (self.va.align_down(page), self.end_va().align_up(page))
    }

    /// Pages installed, counting a partial page at either end as a whole one.
    ///
    /// Derived from [`footprint`](Self::footprint), so the count and the extent cannot
    /// disagree about how far the region reaches.
    pub fn pages(&self) -> usize {
        let (start, end) = self.footprint();
        end.sub_addr(start) / self.page_size()
    }

    /// True when there is nothing to map: a region a platform's geometry collapses to
    /// zero, rather than one the layout has to leave out.
    pub fn is_empty(&self) -> bool { self.len == 0 }

    /// Exclusive end of the virtual range, before page rounding.
    fn end_va(&self) -> VirtualAddr { self.va.add(self.len) }

    /// Reject a region the kernel must never install.
    ///
    /// [`Mapper::map_range_at_level`] rounds outward, so a misaligned superpage region
    /// pulls in its neighbourhood — here, OpenSBI's memory, which PMP denies to S-mode.
    fn validate(&self) {
        let writable = self.flags.contains(PteFlags::WRITE);
        let executable = self.flags.contains(PteFlags::EXECUTE);
        assert!(
            !(writable && executable),
            "region '{}' would be both writable and executable",
            self.name
        );

        let page = self.page_size();
        assert!(
            self.va.is_aligned(page),
            "region '{}' virtual base {:#x} is not aligned to its {page:#x}-byte page",
            self.name,
            self.va
        );
        assert!(
            self.pa.is_aligned(page),
            "region '{}' physical base {:#x} is not aligned to its {page:#x}-byte page",
            self.name,
            self.pa
        );
    }

    /// Install every page of this region into `mapper`.
    pub fn install<F: FrameSource, A: PhysAccess>(
        &self,
        mapper: &mut Mapper<'_, F, A>,
    ) -> Result<(), MapError> {
        if self.is_empty() {
            return Ok(());
        }
        self.validate();

        // One difference for the whole region, so the walk needs no per-page bookkeeping.
        // The types come off because a VA minus a PA is an address of neither kind; they
        // go back on in the closure. Wrapping, since a direct-map VA exceeds its PA.
        let delta = self.pa.bits().wrapping_sub(self.va.bits());
        mapper.map_range_at_level(
            self.va,
            self.end_va(),
            self.level,
            |vaddr| PhysicalAddr::new(vaddr.bits().wrapping_add(delta)),
            self.flags,
        )
    }

    /// Walk every page of this region and require the right level, rights and frame.
    ///
    /// Every page, not a sample: this runs once, and a wrong leaf anywhere is either a
    /// fault or a silent protection hole.
    pub fn audit<F: FrameSource, A: PhysAccess>(&self, mapper: &Mapper<'_, F, A>) {
        if self.is_empty() {
            return;
        }
        let page = self.page_size();
        for index in 0..self.pages() {
            let offset = index * page;
            let vaddr = self.va.add(offset);
            let (entry, level) = mapper
                .entry_of(vaddr)
                .unwrap_or_else(|| panic!("region '{}' left {vaddr:#x} unmapped", self.name));

            assert_eq!(
                level, self.level,
                "region '{}' mapped {vaddr:#x} at level {level}, expected level {}",
                self.name, self.level
            );
            assert_eq!(
                entry.flags(),
                self.flags | PteFlags::VALID,
                "region '{}' has the wrong rights at {vaddr:#x}",
                self.name
            );
            assert_eq!(
                mapper.translate(vaddr),
                Some(self.pa.add(offset)),
                "region '{}' translates {vaddr:#x} to the wrong frame",
                self.name
            );
        }
    }
}

/// Require a region list to tile the address space rather than overlap it.
///
/// Overlap is not an error the hardware reports: the second [`Region::install`] wins and
/// the loser's rights vanish. Compared as [`footprint`](Region::footprint)s, since that is
/// what gets written — two device windows a few hundred bytes apart are disjoint ranges
/// and the same page.
///
/// Mechanism, so it lives beside `install` rather than beside any one layout: a user
/// address space needs the same check over a different list. `O(n²)`, once, over tens of
/// entries.
pub fn audit_disjoint(regions: &[Region<'_>]) {
    for (index, a) in regions.iter().enumerate() {
        if a.is_empty() {
            continue;
        }
        let (a_start, a_end) = a.footprint();
        for b in regions[index + 1..].iter().filter(|b| !b.is_empty()) {
            let (b_start, b_end) = b.footprint();
            assert!(
                a_end <= b_start || b_end <= a_start,
                "regions '{}' ({a_start:#x}..{a_end:#x}) and '{}' ({b_start:#x}..{b_end:#x}) \
                 overlap once rounded to their page size; one would silently replace the \
                 other's rights",
                a.name,
                b.name
            );
        }
    }
}

/// Print a region list as a memory map.
///
/// Puts the protection policy in the boot log, where it can be read off a failing run.
/// The rights come from [`PteFlags::rwx`].
///
/// Adjacent regions sharing a name, page size and rights collapse into one `xN` line —
/// stacks are one region each, so a big machine would otherwise bury everything else.
/// All three must match or the printed total is a lie.
pub fn report(regions: &[Region<'_>]) {
    let mut index = 0;
    while index < regions.len() {
        let region = &regions[index];
        if region.is_empty() {
            index += 1;
            continue;
        }

        // Every field below is load-bearing: without the address checks, scattered MMIO
        // windows sharing a name would print as one contiguous mapping.
        let page_size = region.page_size();
        let mut run = 1;
        let mut pages = region.pages();
        while let Some(next) = regions.get(index + run) {
            // Against the accumulated [`Region::footprint`], not the requested extents:
            // a run is contiguous when each region starts where the last one's pages stop.
            let span = pages * page_size;
            if next.name != region.name
                || next.level != region.level
                || next.flags != region.flags
                || next.is_empty()
                || next.va != region.va.add(span)
                || next.pa != region.pa.add(span)
            {
                break;
            }
            pages += next.pages();
            run += 1;
        }

        println!(
            "[memory]   {:<22} {:#018x} -> {:#012x}  {} {:>5} x {}{}",
            region.name,
            region.va,
            region.pa,
            region.flags.rwx(),
            pages,
            ByteSize(region.page_size()),
            Run(run)
        );
        index += run;
    }
}

/// Renders ` (xN)` for a collapsed run, and nothing for a single region.
struct Run(usize);

impl core::fmt::Display for Run {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.0 > 1 { write!(f, " (x{})", self.0) } else { Ok(()) }
    }
}
