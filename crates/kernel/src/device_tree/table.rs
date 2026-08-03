//! What one walk of the device tree found, and the one place it is kept.
//!
//! # Why this is a struct and not eleven atomics
//!
//! It used to be eleven `AtomicUsize`s next to a separate `Once<Discovered>`, and
//! every device was therefore stored **twice**: `serial@10000000` went into
//! `UART_BASE`/`UART_SIZE` via one tree walk *and*, independently, into the MMIO
//! window list via another. Two representations of one fact, populated by two
//! parsers, with nothing keeping them in step.
//!
//! That was not merely untidy. `kernel_table` maps only the window list, while
//! `console` reads only `uart_base()`, so a tree with more windows than the list
//! holds would drop the UART window from the mapping while `UART_BASE` still
//! pointed at it — a store page fault on the first `println!` after the `satp`
//! switch. The overflow warning fires, but the failure is not the warning's shape.
//!
//! With one table there is one answer, and `uart()` is a lookup rather than a
//! second parse.

use heapless::Vec;

use super::region::PhysRegion;

/// MMIO windows recordable. QEMU virt describes seventeen.
pub const MAX_MMIO: usize = 48;

/// Foreign RAM ranges recordable: `/reserved-memory` nodes, FDT reservation-block
/// entries, the initrd and the blob itself.
pub const MAX_FOREIGN: usize = 24;

/// Hart ids recordable. Independent of how many the kernel has stacks for — the
/// machine reports what exists, `memory::stack` decides how many we can serve.
pub const MAX_HART_IDS: usize = 64;

/// A device the kernel knows by name, resolved from a single node.
///
/// `base`, `size` and `irq` all come off the **same** node. They used to be found
/// by separate searches — one for `reg`, one for `interrupts` — each returning the
/// first *compatible* node that happened to carry the property it wanted. A tree
/// with a debug UART lacking `interrupts` and a real one carrying it would resolve
/// the base from one node and the IRQ from the other, wiring the console to the
/// wrong interrupt line.
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
    pub base: usize,
    /// Exclusive end — the authoritative RAM top.
    pub end: usize,
}

/// Everything one pass over the tree turns up.
pub struct DeviceTable {
    /// The blob itself: where it is and how big, so it can be reserved.
    pub blob: PhysRegion,
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
    /// Every MMIO window the tree describes.
    pub mmio: Vec<PhysRegion, MAX_MMIO>,
    /// Every RAM range that exists but is not the kernel's to hand out.
    pub foreign: Vec<PhysRegion, MAX_FOREIGN>,
    /// Every hart id the machine reports.
    pub hart_ids: Vec<usize, MAX_HART_IDS>,
    /// Nodes skipped for `status = "disabled"`, reported by [`super::summary`].
    pub disabled: usize,
}

/// The one copy. Written once by [`super::init`], read by everything else.
pub static TABLE: spin::Once<DeviceTable> = spin::Once::new();

/// The table, or `None` before the tree has been parsed.
pub fn get() -> Option<&'static DeviceTable> {
    TABLE.get()
}
