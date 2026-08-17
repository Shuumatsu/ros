//! What the kernel must be told about physical memory before it can manage any.
//!
//! This module is the seam, and nothing else: one type, its bounds, and the check that
//! rejects a machine this kernel cannot describe. `memory` does not read the device tree,
//! or anything else about the platform — it is handed one [`MachineMemory`] and works from
//! that, so the subsystem depends on a *description* of the machine rather than on whoever
//! discovered it: [`crate::device_tree`] today, a hand-written table on a board without an
//! FDT tomorrow.
//!
//! One seam also means one place to reject such a machine. [`MachineMemory::check`] does
//! that, rather than each of the three modules that would otherwise have aliased a physical
//! address and found out separately.
//!
//! A description is allowed to be redundant: both lists may name the same memory twice.
//! Removing the overlap belongs to whoever acts on it, since its own page rounding
//! manufactures more — [`super::phys_range::coalesce`] for device windows,
//! `super::frame::reserve` for foreign RAM.

use super::direct_map;
use super::phys_range::PhysRange;

/// Device windows describable. QEMU virt reports seventeen.
pub const MAX_MMIO: usize = 48;

/// Foreign RAM ranges describable: firmware carve-outs, an initrd, a device-tree blob.
///
/// `super::frame::reserve` sizes its record of what it withheld from this, since every
/// range here may produce one. Stated once so the two cannot disagree about how many the
/// kernel can survive.
pub const MAX_FOREIGN: usize = 24;

/// The machine's physical memory, as the kernel needs to see it.
///
/// Borrowed rather than owned: the discoverer keeps the storage, and this is a view of it
/// that [`super::init_allocators`] hands on to the modules that each need one part.
#[derive(Clone, Copy, Debug)]
pub struct MachineMemory<'a> {
    /// The RAM bank the kernel was loaded into, base included — the authoritative extent,
    /// and the only bank this kernel manages.
    ///
    /// The base is carried rather than assumed because it is what makes "the kernel image
    /// is inside the RAM it was told about" and "no device window overlaps RAM" checkable;
    /// [`super::init_allocators`] does both.
    pub ram: &'a PhysRange,
    /// RAM that exists but is not the kernel's to hand out. Overlap between entries is
    /// expected; `super::frame::reserve` owns making them disjoint.
    pub foreign: &'a [PhysRange],
    /// Every device window the platform describes, driven today or not. Overlap between
    /// entries is expected; [`super::phys_range::coalesce`] owns making them disjoint.
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
    ///   [`direct_map::phys_to_virt`] and find unmapped — or worse, colliding with what
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
