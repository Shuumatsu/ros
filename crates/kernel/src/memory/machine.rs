//! Physical-memory description supplied by the platform.
//!
//! Foreign and MMIO ranges may overlap; consumers resolve overlaps at page granularity.

use super::direct_map;
use super::phys_range::PhysRange;

/// Maximum device windows in a machine description.
pub const MAX_MMIO: usize = 48;

/// Maximum foreign RAM carve-outs in a machine description.
pub const MAX_FOREIGN: usize = 24;

/// Physical memory relevant to the kernel.
#[derive(Clone, Copy, Debug)]
pub struct MachineMemory<'a> {
    /// The only RAM bank managed by the kernel.
    pub ram: &'a PhysRange,
    /// RAM excluded from allocation; entries may overlap.
    pub foreign: &'a [PhysRange],
    /// Device windows; entries may overlap.
    pub mmio: &'a [PhysRange],
}

impl MachineMemory<'_> {
    /// Validate capacities, RAM presence, and MMIO reach and disjointness.
    ///
    /// # Panics
    ///
    /// Panics for empty or unreachable RAM, over-capacity lists, or MMIO outside the direct map
    /// or overlapping RAM. RAM ending above the direct map is allowed and remains unmanaged.
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
        // RAM may end above the window, but its base must be addressable.
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
