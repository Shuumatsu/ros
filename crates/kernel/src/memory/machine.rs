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

use heapless::String;

use paging::PhysicalAddr;

use crate::memory::direct_map::DIRECT_MAP_END;
use crate::utils::{ByteSize, truncated};

/// Longest label kept for a range. Device-tree node names reach ~20 characters
/// (`virtio_mmio@10008000`).
pub const NAME_LEN: usize = 40;

/// Device windows describable. QEMU virt reports seventeen.
pub const MAX_MMIO: usize = 48;

/// Foreign RAM ranges describable: firmware carve-outs, an initrd, a device-tree blob.
///
/// Also the size of `super::frame::reserve`'s record of what it withheld, since every
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
}

/// The machine's physical memory, as the kernel needs to see it.
///
/// Borrowed rather than owned: the discoverer keeps the storage, and this is a view of it
/// that [`super::init`] hands on to the modules that each need one part.
#[derive(Clone, Copy, Debug)]
pub struct MachineMemory<'a> {
    /// Exclusive top of the RAM bank the kernel was loaded into — the authoritative RAM
    /// top, and the only bank this kernel manages.
    pub ram_end: PhysicalAddr,
    /// RAM that exists but is not the kernel's to hand out. Overlap between entries is
    /// expected; `super::frame::reserve` owns making them disjoint.
    pub foreign: &'a [PhysRange],
    /// Every device window the platform describes, driven today or not.
    pub mmio: &'a [PhysRange],
}

impl MachineMemory<'_> {
    /// Reject a machine this kernel's address-space layout cannot describe.
    ///
    /// Only device windows are fatal. RAM above the direct map is *dropped* by
    /// [`super::frame::init`], which costs memory and nothing else, but a device the kernel
    /// cannot alias is an address some future driver would take from `phys_to_virt` and
    /// find unmapped — and, worse, one that collides with what [`super::kernel_va`] hands
    /// out. Failing here names the window and the constant to raise.
    pub fn check(&self) {
        for window in self.mmio {
            assert!(
                window.end() <= DIRECT_MAP_END,
                "device window '{}' at {:#x}..{:#x} lies past the direct map's {} window; \
                 raise memory::direct_map::DIRECT_MAP_SPAN",
                window.name(),
                window.base,
                window.end(),
                ByteSize(DIRECT_MAP_END.bits())
            );
        }
    }
}
