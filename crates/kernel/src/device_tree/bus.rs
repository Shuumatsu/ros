//! FDT bus ancestry and `ranges` address translation.

use fdt_raw::{Node, Property};
use heapless::String;

use crate::utils::truncated;

pub const PATH_LEN: usize = 128;

// `fdt_raw` silently corrupts paths and inherited cell counts beyond its 16-entry stack.
const MAX_DEPTH: usize = 16;

pub fn require_depth(node: &Node<'_>) {
    assert!(
        node.level() < MAX_DEPTH,
        "[dtb] node '{}' sits at depth {}, and a path is only tracked {MAX_DEPTH} deep",
        node.name(),
        node.level()
    );
}

/// Test strict ancestry on path-component boundaries; root returns false.
pub fn is_below(ancestor: &str, path: &str) -> bool {
    path.len() > ancestor.len()
        && path.starts_with(ancestor)
        && path.as_bytes()[ancestor.len()] == b'/'
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
    path: String<PATH_LEN>,
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
                // An incomplete entry cannot establish a mapping.
                return None;
            };
            entries += 1;
            if let Some(offset) = address.checked_sub(child)
                && offset < length
            {
                return Some(parent.wrapping_add(offset));
            }
        }

        // Only an actually empty property is an identity mapping. Invalid or unmatched
        // non-empty properties provide no mapping.
        (entries == 0 && ranges.is_empty()).then_some(address)
    }
}

/// Translate a child-bus address through every ancestor into CPU address space.
pub fn to_cpu_address(ancestors: &[Bridge<'_>], address: u64) -> Option<u64> {
    ancestors.iter().rev().try_fold(address, |address, bridge| bridge.translate(address))
}

/// Ancestor buses for a depth-first traversal, nearest last.
pub struct BusStack<'a> {
    bridges: heapless::Vec<Bridge<'a>, MAX_DEPTH>,
    /// Root cell count, retained because the root is excluded from the bridge stack.
    root_address_cells: usize,
}

impl<'a> BusStack<'a> {
    pub fn new() -> Self { Self { bridges: heapless::Vec::new(), root_address_cells: 2 } }

    /// Enter `node` and return its ancestor bridges, excluding the node itself. The caller
    /// must enforce [`require_depth`] before reading `path`.
    pub fn enter(&mut self, node: &Node<'a>, path: &str) -> &[Bridge<'a>] {
        while self.bridges.last().is_some_and(|bridge| !is_below(&bridge.path, path)) {
            self.bridges.pop();
        }
        if path == "/" {
            self.root_address_cells = node.address_cells as usize;
        }
        let bridge = Bridge {
            path: truncated(path),
            ranges: node.find_property("ranges"),
            child_cells: node.address_cells as usize,
            parent_cells: self
                .bridges
                .last()
                .map_or(self.root_address_cells, |parent| parent.child_cells),
            size_cells: node.size_cells as usize,
        };
        let Ok(()) = self.bridges.push(bridge) else {
            unreachable!("require_depth bounds a node and its ancestors to MAX_DEPTH")
        };
        &self.bridges[..self.bridges.len() - 1]
    }
}
