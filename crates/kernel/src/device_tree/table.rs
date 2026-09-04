//! The published hardware table and the device record it holds.

use heapless::Vec;
use mmu::PhysicalAddr;

use crate::cpu::MAX_CPUS;
use crate::memory::machine::{MAX_FOREIGN, MAX_MMIO};
use crate::memory::phys_range::PhysRange;

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
    pub timebase_hz: Option<u64>,
    /// All enabled MMIO windows found during traversal, including unsupported devices.
    /// Entries may overlap.
    pub mmio: Vec<PhysRange, MAX_MMIO>,
    /// RAM excluded from allocation: `/reserved-memory`, header reservations, initrd, and
    /// the FDT blob. Entries may overlap.
    pub foreign: Vec<PhysRange, MAX_FOREIGN>,
    /// Enabled firmware-reported hart IDs, bounded by the cpu slots this kernel has.
    pub hart_ids: Vec<usize, MAX_CPUS>,
    /// Disabled nodes skipped, including descendants of disabled nodes.
    pub disabled: usize,
}

static TABLE: spin::Once<DeviceTable> = spin::Once::new();

/// Publish the table built by `discover` and return it. Later calls return the first table.
pub fn publish(discover: impl FnOnce() -> DeviceTable) -> &'static DeviceTable {
    TABLE.call_once(discover)
}

pub fn get() -> Option<&'static DeviceTable> { TABLE.get() }
