use heapless::Vec;
use mmu::PhysicalAddr;

use crate::cpu::MAX_CPUS;
use crate::memory::machine::{MAX_FOREIGN, MAX_MMIO};
use crate::memory::phys_range::PhysRange;

pub const MAX_HART_IDS: usize = MAX_CPUS;

/// A device resolved from one node.
#[derive(Clone, Copy, Debug)]
pub struct Device {
    /// Base of the first usable `reg` window.
    pub base: PhysicalAddr,
    /// Nonzero window length.
    pub size: usize,
    /// Single-cell interrupt value, or `None` when absent or unsupported.
    pub irq: Option<usize>,
}

pub struct DeviceTable {
    pub blob: PhysRange,
    /// The `/memory` bank containing the kernel and managed by this kernel.
    pub ram: PhysRange,
    pub uart: Device,
    pub timebase_hz: Option<usize>,
    /// All enabled MMIO windows found during traversal, including unsupported devices.
    /// Entries may overlap.
    pub mmio: Vec<PhysRange, MAX_MMIO>,
    /// RAM excluded from allocation: `/reserved-memory`, header reservations, initrd, and
    /// the FDT blob. Entries may overlap.
    pub foreign: Vec<PhysRange, MAX_FOREIGN>,
    pub hart_ids: Vec<usize, MAX_HART_IDS>,
    /// Disabled nodes skipped, including descendants of disabled nodes.
    pub disabled: usize,
}

pub static TABLE: spin::Once<DeviceTable> = spin::Once::new();

pub fn get() -> Option<&'static DeviceTable> { TABLE.get() }
