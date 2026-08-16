//! What the kernel must be told about physical memory before it can manage any.
//!
//! This module is the seam. `memory` does not read the device tree, or anything else about
//! the platform: it is handed one [`MachineMemory`] and works from that, so the subsystem
//! depends on a *description* of the machine rather than on whoever discovered it —
//! [`crate::device_tree`] today, a hand-written table on a board without an FDT tomorrow.
//!
//! One seam also means one place to reject a machine this kernel cannot describe.
//! [`MachineMemory::check`] does that, rather than each of the three modules that would
//! otherwise have aliased a physical address and found out separately.
//!
//! A description is allowed to be redundant: both lists may name the same memory twice.
//! Removing the overlap belongs to whoever acts on it, since its own page rounding
//! manufactures more — [`coalesce`] for device windows, `super::frame::reserve` for
//! foreign RAM.

use alloc::vec::Vec;

use heapless::String;

use paging::sv39::PAGE_SIZE;
use paging::{MemoryAddr, PhysicalAddr};

use crate::memory::direct_map;
use crate::utils::truncated;

/// Longest label kept for a range. Device-tree node names reach ~20 characters
/// (`virtio_mmio@10008000`).
pub const NAME_LEN: usize = 40;

/// Device windows describable. QEMU virt reports seventeen.
pub const MAX_MMIO: usize = 48;

/// Foreign RAM ranges describable: firmware carve-outs, an initrd, a device-tree blob.
///
/// `super::frame::reserve` sizes its record of what it withheld from this, since every
/// range here may produce one. Stated once so the two cannot disagree about how many the
/// kernel can survive.
pub const MAX_FOREIGN: usize = 24;

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
    pub fn frame_span(&self) -> (PhysicalAddr, PhysicalAddr) {
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
/// discoverer's to have avoided.
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
        let (start, end) = window.frame_span();
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

/// The machine's physical memory, as the kernel needs to see it.
///
/// Borrowed rather than owned: the discoverer keeps the storage, and this is a view of it
/// that [`super::init`] hands on to the modules that each need one part.
#[derive(Clone, Copy, Debug)]
pub struct MachineMemory<'a> {
    /// The RAM bank the kernel was loaded into, base included — the authoritative extent,
    /// and the only bank this kernel manages.
    ///
    /// The base is carried rather than assumed because it is what makes "the kernel image
    /// is inside the RAM it was told about" and "no device window overlaps RAM" checkable;
    /// [`super::init`] does both.
    pub ram: &'a PhysRange,
    /// RAM that exists but is not the kernel's to hand out. Overlap between entries is
    /// expected; `super::frame::reserve` owns making them disjoint.
    pub foreign: &'a [PhysRange],
    /// Every device window the platform describes, driven today or not. Overlap between
    /// entries is expected; [`coalesce`] owns making them disjoint.
    pub mmio: &'a [PhysRange],
}

impl MachineMemory<'_> {
    /// Reject a machine this kernel cannot describe, before anything derives an address
    /// from it.
    ///
    /// Ways a description is unusable, all fatal because each one ends in a mapping that is
    /// individually valid and points at the wrong memory:
    ///
    /// - a device window past the direct map, which some future driver would take from
    ///   `phys_to_virt` and find unmapped — or worse, colliding with what
    ///   [`super::kernel_va`] hands out;
    /// - a device window overlapping the RAM bank, which is either register space the
    ///   frame allocator would vend or RAM a driver would write registers into;
    /// - an empty RAM bank;
    /// - more entries in either list than the fixed storage downstream holds. The lists
    ///   arrive as slices, so their bound is not a property of the type; checked here so
    ///   the `heapless` capacities [`super::frame`] copies them into are provable rather
    ///   than hopeful.
    ///
    /// RAM *above* the direct map is not fatal: [`super::frame::init`] drops it loudly,
    /// which costs memory and nothing else.
    pub fn check(&self) {
        assert!(self.ram.size > 0, "the machine reports a RAM bank of no size at all");
        assert!(
            self.mmio.len() <= MAX_MMIO,
            "the machine describes {} device windows, more than the {MAX_MMIO} this kernel \
             can hold",
            self.mmio.len()
        );
        assert!(
            self.foreign.len() <= MAX_FOREIGN,
            "the machine describes {} foreign RAM ranges, more than the {MAX_FOREIGN} this \
             kernel can hold",
            self.foreign.len()
        );
        // The base only: RAM past the window's end is dropped rather than rejected, but a
        // bank starting past it leaves the kernel with no addressable memory at all.
        direct_map::require_reach("the base of the kernel's RAM bank", self.ram.base, 1);

        for window in self.mmio {
            direct_map::require_reach(window.name(), window.base, window.size);
            assert!(
                !window.overlaps(self.ram),
                "device window '{}' at {:#x}..{:#x} overlaps the RAM bank at {:#x}..{:#x}; \
                 the device tree describes one range as both register space and memory",
                window.name(),
                window.base,
                window.end(),
                self.ram.base,
                self.ram.end()
            );
        }
    }
}
