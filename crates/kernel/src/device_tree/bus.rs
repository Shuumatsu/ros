//! Where a node sits in the tree, and what the addresses in its `reg` mean.
//!
//! A `reg` is written in the address space its parent bus publishes, so turning one into the
//! address the CPU issues takes the whole chain of buses above it. `all_nodes()` is flat, so
//! [`BusStack`] rebuilds that chain from depth-first order.
//!
//! `fdt_raw` has a `ranges` parser of its own, but it counts an entry's length in the
//! *parent's* `#size-cells`, where the spec says the length is counted in the `#size-cells`
//! of the node carrying `ranges`. It also has no guard against `#size-cells = <0>`, on which
//! its iterator consumes nothing per entry and never ends. Hence [`Bridge`].

use fdt_raw::{Node, Property};
use heapless::String;

use crate::utils::truncated;

/// Longest node path kept, for the bus stack and the console lookup. Paths are compared by
/// prefix, so confusing two nodes takes a pathological tree — `/soc/virtio_mmio@10008000`
/// is 26 characters.
pub const PATH_LEN: usize = 128;

/// Deepest node this walk accepts, which is the tree's depth rather than its width.
///
/// `fdt_raw` tracks a node's path and its inherited cell counts in stacks of this depth
/// whose pushes fail silently while the matching pops do not, so from here down a node
/// reports another node's path — and this walk classifies by path. The bus stack below holds
/// a node and its ancestors, so the same bound covers it and its push cannot fail.
const MAX_DEPTH: usize = 16;

/// Refuse a node deeper than a path can be tracked.
///
/// Called before a path is read rather than before it is translated: from [`MAX_DEPTH`] down
/// a node reports another node's path, and the walk classifies by path.
pub fn require_depth(node: &Node<'_>) {
    assert!(
        node.level() < MAX_DEPTH,
        "[dtb] node '{}' sits at depth {}, and a path is only tracked {MAX_DEPTH} deep",
        node.name(),
        node.level()
    );
}

/// Whether `path` names a node strictly below `ancestor`.
///
/// The component boundary matters: `/soc-foo` is not below `/soc`. The root is never an
/// ancestor by this test, which is what it should be — its children's `reg` are already CPU
/// addresses, so there is nothing to translate through.
pub fn is_below(ancestor: &str, path: &str) -> bool {
    path.len() > ancestor.len()
        && path.starts_with(ancestor)
        && path.as_bytes()[ancestor.len()] == b'/'
}

/// Take `count` big-endian cells as one number, or `None` if the property runs out.
///
/// One or two cells only, both being firmware input: zero consumes nothing and still yields
/// a value, which a `ranges` walk would spin on forever, and three is an address wider than
/// the 64 bits carried here.
fn take_cells(cells: &mut impl Iterator<Item = u32>, count: usize) -> Option<u64> {
    if !(1..=2).contains(&count) {
        return None;
    }
    (0..count).try_fold(0u64, |value, _| Some((value << 32) | u64::from(cells.next()?)))
}

/// A parent bus, and how it maps its children's addresses into its own space.
///
/// The cell counts are carried rather than read back off the node because one `ranges` entry
/// is laid out by three of them, from two different nodes: a child address in *this* bus's
/// `#address-cells`, a parent address in its parent's, and a length in this bus's
/// `#size-cells`. The stack knows all three because it holds the chain; a node on its own
/// does not.
pub struct Bridge<'a> {
    path: String<PATH_LEN>,
    /// The node's `ranges`. `None` means the property is absent, which per the spec means
    /// the child address space is *not* mapped into the parent's at all — a very different
    /// thing from an empty `ranges`, which means it maps one-to-one.
    ranges: Option<Property<'a>>,
    /// Cells of a child address on this bus, which is also its `#address-cells`.
    child_cells: usize,
    /// Cells of a parent address, i.e. the parent bus's `#address-cells`.
    parent_cells: usize,
    /// Cells of a length on this bus.
    size_cells: usize,
}

impl Bridge<'_> {
    /// Carry one child-bus address into this bus's own space, or `None` if this bus does not
    /// map it.
    fn translate(&self, address: u64) -> Option<u64> {
        let ranges = self.ranges.as_ref()?;
        let mut cells = ranges.as_u32_iter();
        let mut entries = 0;

        while let Some(child) = take_cells(&mut cells, self.child_cells) {
            let (Some(parent), Some(length)) = (
                take_cells(&mut cells, self.parent_cells),
                take_cells(&mut cells, self.size_cells),
            ) else {
                // A truncated entry means the cell counts and the property disagree, so
                // every entry read so far is suspect too.
                return None;
            };
            entries += 1;
            if let Some(offset) = address.checked_sub(child)
                && offset < length
            {
                return Some(parent.wrapping_add(offset));
            }
        }

        // An empty `ranges` maps its children one-to-one; a non-empty one that matched
        // nothing does not map this address at all. The property is asked rather than the
        // entry count, which also reads zero when the cell counts are unreadable or the
        // property is truncated.
        (entries == 0 && ranges.is_empty()).then_some(address)
    }
}

/// Carry a child-bus address all the way up to the address the CPU issues.
///
/// A `reg` is written in the address space its parent bus publishes, and only an identity
/// `ranges` at every level makes that the CPU's.
pub fn to_cpu_address(ancestors: &[Bridge<'_>], address: u64) -> Option<u64> {
    ancestors.iter().rev().try_fold(address, |address, bridge| bridge.translate(address))
}

/// The buses above the node being visited, nearest last.
///
/// Depth-first order is what lets one stack stand in for the parent pointer a flat iterator
/// does not give us: a node is visited after its parent and before its parent's next
/// sibling, so popping everything the current path is not below leaves exactly its chain.
pub struct BusStack<'a> {
    bridges: heapless::Vec<Bridge<'a>, MAX_DEPTH>,
    /// The root's `#address-cells`. [`is_below`] never keeps the root, so it is never on the
    /// stack when its children are visited — yet its `#address-cells` is what a top-level
    /// node's `ranges` counts a parent address in.
    root_address_cells: usize,
}

impl<'a> BusStack<'a> {
    /// An empty stack, with the spec's default `#address-cells` until `/` says otherwise,
    /// which it does before any other node.
    pub fn new() -> Self { Self { bridges: heapless::Vec::new(), root_address_cells: 2 } }

    /// Enter `node`, and yield the chain of buses its `reg` has to climb.
    ///
    /// The node itself goes on the stack for whichever of its children comes next, and is
    /// left out of what is returned, so a `reg` is not translated through its own bus.
    ///
    /// # Panics
    ///
    /// If the node is deeper than [`require_depth`] allows, which the caller must have
    /// rejected before reading `path`.
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
