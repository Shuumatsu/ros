//! A named physical address range taken from a device-tree node.

use crate::utils::truncated;

/// Longest device-tree node name recorded. `virtio_mmio@10008000` is 20 characters.
pub const NAME_LEN: usize = 40;

/// A named physical address range taken from a device-tree node's `reg`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysRegion {
    /// Device-tree node name, e.g. `serial@10000000`. Copied rather than borrowed
    /// so the list outlives the parse.
    name: heapless::String<NAME_LEN>,
    /// Physical base of the range.
    pub base: usize,
    /// Length in bytes.
    pub size: usize,
}

impl PhysRegion {
    /// Build a region, truncating an over-long label but never the range. See
    /// [`truncated`] for why the name is not byte-sliced.
    pub fn new(name: &str, base: usize, size: usize) -> Self {
        Self { name: truncated(name), base, size }
    }

    /// The device-tree node name this range came from.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Exclusive end of the range.
    pub fn end(&self) -> usize {
        self.base.saturating_add(self.size)
    }
}
