//! A named physical address range, and the geometry of a list of them.
//!
//! The kernel's one description of "this much physical memory, and what it is": the RAM
//! bank and the firmware's carve-outs as the platform reports them, the kernel image as
//! the linker laid it out, a device window, the frame pool. Its own module because it
//! belongs to none of those — filing it under whichever one needed it first is what makes
//! the kernel image look like a fact somebody discovered about the machine.
//!
//! Geometry only. Nothing here knows what a range is *for*, which is why the same type
//! survives being withheld from the frame pool, mapped into a page table, and printed.
//! [`Region`](super::region::Region) is the virtual counterpart and shares its vocabulary:
//! both round outward to whole pages and call the result a [`footprint`](PhysRange::footprint).

use alloc::vec::Vec;

use heapless::String;

use mmu::PAGE_SIZE;
use mmu::{MemoryAddr, PhysicalAddr};

use crate::utils::truncated;

/// Longest label kept for a range. Device-tree node names reach ~20 characters
/// (`virtio_mmio@10008000`).
pub const NAME_LEN: usize = 40;

/// A named physical address range.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysRange {
    /// What this range is. Copied rather than borrowed, so the description outlives
    /// whatever was parsed to build it.
    name: String<NAME_LEN>,
    /// First physical address.
    pub base: PhysicalAddr,
    /// Length in bytes.
    pub size: usize,
}

impl PhysRange {
    /// Build a range, truncating an over-long label but never the range itself. See
    /// [`truncated`] for why the name is not byte-sliced.
    pub fn new(name: &str, base: PhysicalAddr, size: usize) -> Self {
        Self { name: truncated(name), base, size }
    }

    /// What this range is, for diagnostics and the boot log.
    pub fn name(&self) -> &str { &self.name }

    /// Exclusive end of the range. Saturating: a firmware-supplied size that would wrap
    /// must not produce an end below the base.
    pub fn end(&self) -> PhysicalAddr {
        PhysicalAddr::new(self.base.bits().saturating_add(self.size))
    }

    /// Whether `address` falls inside the range.
    pub fn contains(&self, address: PhysicalAddr) -> bool {
        self.base <= address && address < self.end()
    }

    /// Whether the two ranges share a byte.
    pub fn overlaps(&self, other: &Self) -> bool {
        self.base < other.end() && other.base < self.end()
    }

    /// The range rounded outward to whole frames: what actually gets withheld or mapped,
    /// since neither a reservation nor a page table can act on part of a page.
    pub fn footprint(&self) -> (PhysicalAddr, PhysicalAddr) {
        (self.base.align_down(PAGE_SIZE), self.end().align_up(PAGE_SIZE))
    }
}

/// Merge a list of physical windows into one whose entries cannot share a page.
///
/// Overlap between device windows is legal in a device tree — a syscon child inside its
/// parent's register block, a node repeating a `reg`, two devices a few hundred bytes
/// apart — and page rounding manufactures more of it. Whoever maps these owns resolving
/// it, for the same reason [`super::frame::reserve`] owns it for foreign RAM: the outward
/// rounding is the mapper's, so the overlap is the mapper's to absorb rather than the
/// discoverer's to have avoided. Hence a free function over a list rather than anything
/// [`super::machine`] does to a description on the way in.
///
/// Only genuine overlap is merged. Windows in adjacent pages already get PTEs of their
/// own, so they stay separate and keep their node names in the boot log; a merged entry
/// is named after its first contributor and says how many joined it.
pub fn coalesce(windows: &[PhysRange]) -> Vec<PhysRange> {
    let mut sorted: Vec<&PhysRange> = windows.iter().filter(|window| window.size > 0).collect();
    sorted.sort_unstable_by_key(|window| window.base);

    // `(first contributor, start, end, how many more joined)`, so a name is composed once
    // at the end rather than rewritten on every merge.
    let mut runs: Vec<(&str, PhysicalAddr, PhysicalAddr, usize)> = Vec::new();
    for window in sorted {
        let (start, end) = window.footprint();
        match runs.last_mut() {
            Some(run) if start < run.2 => {
                run.2 = run.2.max(end);
                run.3 += 1;
            }
            _ => runs.push((window.name(), start, end, 0)),
        }
    }

    runs.into_iter()
        .map(|(name, start, end, joined)| {
            let mut label: String<NAME_LEN> = truncated(name);
            if joined > 0 {
                let _ = core::fmt::Write::write_fmt(&mut label, format_args!(" +{joined}"));
            }
            PhysRange { name: label, base: start, size: end.sub_addr(start) }
        })
        .collect()
}
