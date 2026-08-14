//! What one walk of the device tree found, and the one place it is kept.
//!
//! One table, not per-device statics: a UART base kept both in its own static and in the
//! MMIO window list could disagree, and since `kernel_table` maps only the list, the
//! console would end up holding an address nothing mapped. The well-known devices are
//! therefore indices into the same walk that produced the list.

use heapless::Vec;

use paging::PhysicalAddr;

use crate::memory::machine::{MAX_FOREIGN, MAX_MMIO, PhysRange};

/// Hart ids recordable. Independent of how many the kernel has stacks for — the
/// machine reports what exists, `memory::stack` decides how many we can serve.
pub const MAX_HART_IDS: usize = 64;

/// A device the kernel knows by name, resolved from a single node.
///
/// `base`, `size` and `irq` must come off the **same** node: searching per property finds
/// the first compatible node carrying *that* property, so a debug UART without
/// `interrupts` beside a real one would yield a base from one and an IRQ from the other.
#[derive(Clone, Copy, Debug)]
pub struct Device {
    pub base: usize,
    pub size: usize,
    /// `None` when the node declared no `interrupts`.
    pub irq: Option<usize>,
}

/// Physical RAM backing the kernel.
#[derive(Clone, Copy, Debug)]
pub struct Ram {
    pub base: PhysicalAddr,
    /// Exclusive end — the authoritative RAM top.
    pub end: PhysicalAddr,
}

/// Everything one pass over the tree turns up.
pub struct DeviceTable {
    /// The blob itself: where it is and how big, so it can be reserved.
    pub blob: PhysRange,
    /// The `/memory` bank containing the kernel.
    pub ram: Ram,
    /// The primary console UART. Not optional: without it there is no console, and
    /// [`super::init`] panics rather than let the kernel limp on a guessed address.
    pub uart: Device,
    /// Platform-level interrupt controller, if the tree described one.
    pub plic: Option<Device>,
    /// Core-local interruptor, if the tree described one.
    pub clint: Option<Device>,
    /// Ticks per second of the `time` CSR, from `/cpus/timebase-frequency`.
    pub timebase_hz: Option<usize>,
    /// Every MMIO window the tree describes — the single answer to "where is device
    /// memory", and a genuine walk rather than a list of the devices driven today, so a
    /// future driver finds its window here.
    ///
    /// A window says the *device* exists, not that S-mode may touch it: PMP is a separate
    /// layer, and denies the CLINT on QEMU virt.
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
