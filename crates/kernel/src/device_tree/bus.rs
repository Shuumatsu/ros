//! FDT bus ancestry and `ranges` address translation.

use fdt_raw::{Node, Property};
use heapless::String;

use super::PATH_LEN;

// `fdt_raw` keeps its path and cell context on 16-entry stacks and discards pushes past them
// while still counting the level, so its later pops retire real ancestors. Every node after the
// first one this deep carries a wrong path and wrong inherited cell counts.
const MAX_DEPTH: usize = 16;

/// Return `node`'s absolute path, rejecting depths the parser cannot track.
pub fn path_of(node: &Node<'_>) -> String<PATH_LEN> {
    assert!(
        node.level() < MAX_DEPTH,
        "[dtb] node '{}' sits at depth {}, past the {MAX_DEPTH} the parser tracks; every node \
         after it would report a wrong path and wrong inherited cell counts",
        node.name(),
        node.level()
    );
    node.path()
}

/// Decode one or two big-endian 32-bit cells; reject other widths and truncated values.
fn take_cells(cells: &mut impl Iterator<Item = u32>, count: usize) -> Option<u64> {
    if !(1..=2).contains(&count) {
        return None;
    }
    (0..count).try_fold(0u64, |value, _| Some((value << 32) | u64::from(cells.next()?)))
}

/// A parent bus and its child-to-parent address mapping.
pub struct Bridge<'a> {
    /// An absent property provides no mapping; an empty property is an identity mapping.
    ranges: Option<Property<'a>>,
    child_cells: usize,
    parent_cells: usize,
    size_cells: usize,
}

impl Bridge<'_> {
    fn translate(&self, address: u64) -> Option<u64> {
        let ranges = self.ranges.as_ref()?;
        let mut cells = ranges.as_u32_iter();
        let mut entries = 0;

        while let Some(child) = take_cells(&mut cells, self.child_cells) {
            let (Some(parent), Some(length)) = (
                take_cells(&mut cells, self.parent_cells),
                take_cells(&mut cells, self.size_cells),
            ) else {
                return None;
            };
            entries += 1;
            if let Some(offset) = address.checked_sub(child)
                && offset < length
            {
                return Some(parent.wrapping_add(offset));
            }
        }

        (entries == 0 && ranges.is_empty()).then_some(address)
    }
}

/// Translate a child-bus address through every ancestor into CPU address space.
pub fn to_cpu_address(ancestors: &[Bridge<'_>], address: u64) -> Option<u64> {
    ancestors.iter().rev().try_fold(address, |address, bridge| bridge.translate(address))
}

/// Ancestor buses for a depth-first traversal, nearest last.
///
/// The root is not a bridge: the addresses its children publish are already CPU addresses.
pub struct BusStack<'a> {
    /// The bridge for the entered node at level `index + 1`.
    bridges: heapless::Vec<Bridge<'a>, MAX_DEPTH>,
    /// The root's `#address-cells`, which every level-1 bridge takes as its parent width.
    root_address_cells: usize,
}

impl<'a> BusStack<'a> {
    pub fn new() -> Self { Self { bridges: heapless::Vec::new(), root_address_cells: 2 } }

    /// Enter `node` and return its ancestor bridges, excluding the node itself.
    ///
    /// Depth-first order makes the node's level the height of its own ancestry, so entering it
    /// retires every bridge the previous branch left behind.
    pub fn enter(&mut self, node: &Node<'a>) -> &[Bridge<'a>] {
        let Some(depth) = node.level().checked_sub(1) else {
            self.root_address_cells = node.address_cells as usize;
            self.bridges.clear();
            return &[];
        };

        self.bridges.truncate(depth);
        let ancestors = self.bridges.len();
        let bridge = Bridge {
            ranges: node.find_property("ranges"),
            child_cells: node.address_cells as usize,
            parent_cells: self
                .bridges
                .last()
                .map_or(self.root_address_cells, |parent| parent.child_cells),
            size_cells: node.size_cells as usize,
        };
        let Ok(()) = self.bridges.push(bridge) else {
            unreachable!("path_of bounds a node's level, and its ancestry, to MAX_DEPTH")
        };
        &self.bridges[..ancestors]
    }
}
