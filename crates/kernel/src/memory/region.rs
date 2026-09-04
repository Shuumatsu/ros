//! Installing, auditing, and reporting mapped regions.

use mmu::{
    FrameSource, MapError, Mapper, MemoryAddr, PhysAccess, PhysicalAddr, PteFlags, Scheme,
    VirtualAddr, page_size_at,
};

use crate::utils::ByteSize;

/// A contiguous virtual-to-physical mapping with a constant offset.
#[derive(Clone, Copy)]
pub struct Region<'a> {
    pub name: &'a str,
    pub va: VirtualAddr,
    pub pa: PhysicalAddr,
    /// Length in bytes; zero is a no-op.
    pub len: usize,
    /// Page-table level to build the region from: 0 = 4 KiB, 1 = 2 MiB, 2 = 1 GiB.
    pub level: usize,
    /// Rights excluding `VALID`, which the mapper adds.
    pub flags: PteFlags,
}

impl Region<'_> {
    fn page_size(&self) -> usize { page_size_at(self.level) }

    /// The page-rounded virtual range installed by this region.
    pub fn footprint(&self) -> (VirtualAddr, VirtualAddr) {
        let page = self.page_size();
        (self.va.align_down(page), self.end_va().align_up(page))
    }

    pub fn pages(&self) -> usize {
        let (start, end) = self.footprint();
        end.sub_addr(start) / self.page_size()
    }

    pub fn is_empty(&self) -> bool { self.len == 0 }

    fn end_va(&self) -> VirtualAddr { self.va.add(self.len) }

    /// Enforce W^X and leaf alignment before outward page rounding.
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

    /// Validate and install the region.
    ///
    /// # Panics
    ///
    /// Panics if the mapping is writable and executable or either base is leaf-misaligned.
    pub fn install<S: Scheme, F: FrameSource, A: PhysAccess>(
        &self,
        mapper: &mut Mapper<'_, S, F, A>,
    ) -> Result<(), MapError> {
        if self.is_empty() {
            return Ok(());
        }
        self.validate();

        // Wrapping preserves the constant offset for high-half mappings.
        let delta = self.pa.bits().wrapping_sub(self.va.bits());
        mapper.map_range_at_level(
            self.va,
            self.end_va(),
            self.level,
            |vaddr| PhysicalAddr::new(vaddr.bits().wrapping_add(delta)),
            self.flags,
        )
    }

    /// Verify every leaf's level, rights, and physical translation.
    ///
    /// # Panics
    ///
    /// Panics on any missing or mismatched leaf.
    pub fn audit<S: Scheme, F: FrameSource, A: PhysAccess>(&self, mapper: &Mapper<'_, S, F, A>) {
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

/// Require page-rounded region footprints to be disjoint.
///
/// # Panics
///
/// Panics if any nonempty footprints overlap.
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

/// Print a memory map, collapsing contiguous equivalent regions.
pub fn report(regions: &[Region<'_>]) {
    let mut index = 0;
    while index < regions.len() {
        let region = &regions[index];
        if region.is_empty() {
            index += 1;
            continue;
        }

        let page_size = region.page_size();
        let mut run = 1;
        let mut pages = region.pages();
        while let Some(next) = regions.get(index + run) {
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

struct Run(usize);

impl core::fmt::Display for Run {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.0 > 1 { write!(f, " (x{})", self.0) } else { Ok(()) }
    }
}
