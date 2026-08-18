//! What one walk of the device tree found, and the one place it is kept.
//!
//! One table, not per-device statics: a UART base kept both in its own static and in the
//! MMIO window list could disagree, and since `kernel_table` maps only the list, the
//! console would end up holding an address nothing mapped. A device resolved below
//! therefore carries a window the same walk already put in that list.

use heapless::Vec;
use mmu::PhysicalAddr;

use crate::cpu::MAX_CPUS;
use crate::memory::machine::{MAX_FOREIGN, MAX_MMIO};
use crate::memory::phys_range::PhysRange;

/// Hart ids recordable. The kernel can serve one hart per [`MAX_CPUS`] slot, so recording
/// more would be describing machines it cannot run on; the walk says so where it finds
/// them, rather than letting `cpu::start_secondaries` discover it after `memory::stack`
/// has already allocated and mapped a stack for every one of them.
pub const MAX_HART_IDS: usize = MAX_CPUS;

/// A device the kernel knows by name, resolved from a single node.
///
/// `base`, `size` and `irq` must come off the **same** node: searching per property finds
/// the first compatible node carrying *that* property, so a debug UART without
/// `interrupts` beside a real one would yield a base from one and an IRQ from the other.
#[derive(Clone, Copy, Debug)]
pub struct Device {
    /// First address of the node's first usable `reg` window, which is also an entry in
    /// [`DeviceTable::mmio`] — so a driver reaching it through `phys_to_virt` finds it
    /// mapped.
    pub base: PhysicalAddr,
    /// Length of that window. Never zero: a `reg` without a usable size contributes no
    /// window, and a node that contributes none resolves no device.
    pub size: usize,
    /// The interrupt this device raises, when the tree states it unambiguously — see
    /// `super::walk`'s `irq_of`. `None` also means "the tree did not say plainly", not
    /// only "the device has no interrupt".
    pub irq: Option<usize>,
}

/// Everything one pass over the tree turns up.
pub struct DeviceTable {
    /// The blob itself: where it is and how big, so it can be reserved.
    pub blob: PhysRange,
    /// The `/memory` bank containing the kernel, base and all: the authoritative extent of
    /// the RAM this kernel manages.
    pub ram: PhysRange,
    /// The primary console UART. Not optional: without it there is no console, and
    /// [`super::init`] panics rather than let the kernel limp on a guessed address.
    pub uart: Device,
    /// Ticks per second of the `time` CSR, from `timebase-frequency`.
    pub timebase_hz: Option<usize>,
    /// Every MMIO window the tree describes — the single answer to "where is device
    /// memory", and a genuine walk rather than a list of the devices driven today, so a
    /// future driver finds its window here.
    ///
    /// A window says the *device* exists, not that S-mode may touch it: PMP is a separate
    /// layer, and denies the CLINT on QEMU virt. Overlap between entries is expected;
    /// `memory::phys_range::coalesce` owns removing it, because the page rounding that
    /// creates most of it belongs to whoever maps these.
    pub mmio: Vec<PhysRange, MAX_MMIO>,
    /// Every RAM range that exists but is not the kernel's to hand out; the frame
    /// allocator would otherwise vend all of it.
    ///
    /// Four sources, since honouring some of them is indistinguishable from honouring
    /// none: `/reserved-memory`, the older FDT `off_mem_rsvmap` block (used by U-Boot and
    /// coreboot), `/chosen`'s initrd, and the blob itself. Overlap is expected —
    /// `memory::frame::reserve` owns disjointness, because its outward rounding is what
    /// destroys it.
    pub foreign: Vec<PhysRange, MAX_FOREIGN>,
    /// Every hart id the machine reports.
    pub hart_ids: Vec<usize, MAX_HART_IDS>,
    /// Nodes skipped for `status = "disabled"`, reported by [`super::summary`].
    pub disabled: usize,
}

/// The one copy. Written once by [`super::init`], read by everything else.
pub static TABLE: spin::Once<DeviceTable> = spin::Once::new();

/// The table, or `None` before the tree has been parsed.
pub fn get() -> Option<&'static DeviceTable> { TABLE.get() }
