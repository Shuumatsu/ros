use core::num::NonZeroUsize;

use heapless::Vec;

use crate::bitmap::{Bitmap, WORD_BITS};
use crate::range::FrameRange;

const MAX_ROOTS: usize = usize::BITS as usize * 2;

/// Exact bitmap storage required to manage a frame range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MetadataLayout {
    bits: usize,
    words: usize,
    roots: usize,
}

impl MetadataLayout {
    /// Number of meaningful bitmap bits.
    pub const fn bits(self) -> usize { self.bits }

    /// Number of `usize` words the caller must provide.
    pub const fn words(self) -> usize { self.words }

    /// Number of aligned buddy trees covering the range.
    pub const fn roots(self) -> usize { self.roots }
}

/// Calculate the exact caller-owned storage needed for `range`.
pub fn metadata_layout(range: FrameRange) -> Result<MetadataLayout, MetadataError> {
    decompose(range).map(|(_, layout)| layout)
}

/// Decompose `range` into its aligned buddy roots, assigning each root the bit
/// offset of its tree within the shared bitmap, and derive the [`MetadataLayout`]
/// in the same pass.
///
/// This is the crate's single source of truth for the frame metadata layout:
/// both [`metadata_layout`] and [`FrameAllocator::new`] consume it instead of
/// recomputing the decomposition independently.
fn decompose(range: FrameRange) -> Result<(Vec<Root, MAX_ROOTS>, MetadataLayout), MetadataError> {
    let mut roots = Vec::<Root, MAX_ROOTS>::new();
    let mut bits = 0usize;

    for block in range.roots() {
        let frames = block.frame_count();
        // A buddy tree over `frames` leaves has `2 * frames - 1` nodes.
        let nodes = frames.checked_add(frames - 1).ok_or(MetadataError::CapacityOverflow)?;
        roots
            .push(Root { start: block.start, order: block.order, bit_offset: bits })
            .expect("buddy roots exceed MAX_ROOTS");
        bits = bits.checked_add(nodes).ok_or(MetadataError::CapacityOverflow)?;
    }

    let words = bits.div_ceil(WORD_BITS);
    let layout = MetadataLayout { bits, words, roots: roots.len() };
    Ok((roots, layout))
}

/// Failure to represent the bitmap size on this architecture.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MetadataError {
    /// The range would require more bitmap bits than `usize` can index.
    #[error("frame metadata size exceeds usize")]
    CapacityOverflow,
}

/// Failure to construct a frame allocator.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum InitError {
    /// The required metadata size cannot be represented.
    //
    // `#[error("{0}")]` (not `transparent`) so `source()` resolves to the inner
    // `MetadataError` via `#[from]`, matching the previous hand-written chain.
    #[error("{0}")]
    Metadata(#[from] MetadataError),
    /// The supplied word slice is smaller than [`metadata_layout`] requires.
    #[error("insufficient frame metadata: required {required} words, provided {provided}")]
    InsufficientMetadata {
        /// Exact number of words required.
        required: usize,
        /// Number of words supplied by the caller.
        provided: usize,
    },
}

/// A contiguous power-of-two allocation returned by [`FrameAllocator`].
///
/// The type is intentionally neither `Copy` nor `Clone`: consuming it during
/// deallocation prevents accidental double frees in ordinary safe code.
#[derive(Debug, Eq, PartialEq)]
pub struct FrameBlock {
    start: usize,
    requested_frames: usize,
    order: usize,
    root_index: usize,
    node: usize,
}

impl FrameBlock {
    /// First numeric frame identifier in the allocation.
    pub const fn start_frame(&self) -> usize { self.start }

    /// Number of frames requested by the caller.
    pub const fn requested_frames(&self) -> usize { self.requested_frames }

    /// Number of frames actually reserved after buddy rounding.
    pub const fn frame_count(&self) -> usize { 1usize << self.order }
}

/// Error detected while returning an allocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum DeallocationError {
    /// The block was created by an allocator with a different range layout.
    #[error("frame block does not belong to this allocator")]
    ForeignBlock,
    /// The block, or an ancestor containing it, is already free.
    #[error("frame block is already free")]
    AlreadyFree,
}

#[derive(Clone, Copy, Debug)]
struct Root {
    start: usize,
    order: usize,
    bit_offset: usize,
}

/// A buddy allocator over numeric frame identifiers.
///
/// All mutable bookkeeping lives in a bitmap borrowed from the caller. The
/// allocator does not dereference, clear, or otherwise access managed frames.
/// It is deliberately unsynchronized; a shared owner must provide locking.
pub struct FrameAllocator<'a> {
    range: FrameRange,
    bitmap: Bitmap<'a>,
    roots: Vec<Root, MAX_ROOTS>,
    max_order: usize,
    free_frames: usize,
}

impl<'a> FrameAllocator<'a> {
    /// Construct an allocator owning every frame in `range`.
    ///
    /// The required prefix of `metadata` is cleared and retained for the
    /// allocator's lifetime; additional words are ignored.
    ///
    /// # Safety
    ///
    /// The caller must have exclusive ownership of every frame in `range`.
    /// None may be allocated, managed by another allocator, or contain the
    /// supplied metadata storage. That ownership must remain exclusive until
    /// this allocator is destroyed or all frames are transferred deliberately.
    pub unsafe fn new(range: FrameRange, metadata: &'a mut [usize]) -> Result<Self, InitError> {
        let (roots, layout) = decompose(range)?;
        if metadata.len() < layout.words {
            return Err(InitError::InsufficientMetadata {
                required: layout.words,
                provided: metadata.len(),
            });
        }

        let max_order = roots.iter().map(|root| root.order).max().unwrap_or(0);

        let mut bitmap = Bitmap::new(&mut metadata[..layout.words], layout.bits);
        for root in &roots {
            bitmap.set(root.bit_offset);
        }

        Ok(Self { range, bitmap, roots, max_order, free_frames: range.len() })
    }

    /// Entire frame range managed by this allocator.
    pub const fn range(&self) -> FrameRange { self.range }

    /// Total number of managed frames.
    pub const fn total_frames(&self) -> usize { self.range.len() }

    /// Number of frames currently available.
    pub const fn free_frames(&self) -> usize { self.free_frames }

    /// Number of frames currently reserved, including buddy rounding.
    pub const fn allocated_frames(&self) -> usize { self.total_frames() - self.free_frames }

    /// Allocate a contiguous frame block.
    ///
    /// `count` is rounded up to a power of two. The returned start is aligned
    /// to the resulting [`FrameBlock::frame_count`].
    pub fn allocate(&mut self, count: NonZeroUsize) -> Option<FrameBlock> {
        let requested_frames = count.get();
        let frame_count = requested_frames.checked_next_power_of_two()?;
        let target_order = frame_count.trailing_zeros() as usize;
        if target_order > self.max_order {
            return None;
        }

        for source_order in target_order..=self.max_order {
            for root_index in 0..self.roots.len() {
                let root = self.roots[root_index];
                if source_order > root.order {
                    continue;
                }
                let Some(mut node) = self.find_free_node(root, source_order) else {
                    continue;
                };

                self.bitmap.clear(root.bit_offset + node);
                let mut current_order = source_order;
                while current_order > target_order {
                    let left_child = node * 2 + 1;
                    let right_child = left_child + 1;
                    self.bitmap.set(root.bit_offset + right_child);
                    node = left_child;
                    current_order -= 1;
                }

                let depth = root.order - target_order;
                let first_node = first_node_at_depth(depth);
                let position = node - first_node;
                let start = root.start + position * frame_count;
                self.free_frames -= frame_count;

                return Some(FrameBlock {
                    start,
                    requested_frames,
                    order: target_order,
                    root_index,
                    node,
                });
            }
        }

        None
    }

    /// Return a block previously produced by [`allocate`](Self::allocate).
    ///
    /// # Safety
    ///
    /// The caller must guarantee that no live mapping, pointer, DMA operation,
    /// or other owner can access any frame in `block`. The block must come from
    /// this allocator and must not have been deallocated before.
    pub unsafe fn deallocate(&mut self, block: FrameBlock) -> Result<(), DeallocationError> {
        let root =
            self.roots.get(block.root_index).copied().ok_or(DeallocationError::ForeignBlock)?;
        if !block_matches_root(&block, root) {
            return Err(DeallocationError::ForeignBlock);
        }

        let mut ancestor = block.node;
        loop {
            if self.bitmap.get(root.bit_offset + ancestor) {
                return Err(DeallocationError::AlreadyFree);
            }
            if ancestor == 0 {
                break;
            }
            ancestor = parent(ancestor);
        }

        let frame_count = block.frame_count();
        if frame_count > self.total_frames() - self.free_frames {
            return Err(DeallocationError::AlreadyFree);
        }

        let mut node = block.node;
        while node != 0 {
            let buddy = sibling(node);
            if !self.bitmap.get(root.bit_offset + buddy) {
                break;
            }
            self.bitmap.clear(root.bit_offset + buddy);
            node = parent(node);
        }
        self.bitmap.set(root.bit_offset + node);
        self.free_frames += frame_count;
        Ok(())
    }

    fn find_free_node(&self, root: Root, order: usize) -> Option<usize> {
        let depth = root.order - order;
        let first = first_node_at_depth(depth);
        let end = first + (1usize << depth);
        self.bitmap
            .find_first_set(root.bit_offset + first, root.bit_offset + end)
            .map(|bit| bit - root.bit_offset)
    }
}

fn first_node_at_depth(depth: usize) -> usize { (1usize << depth) - 1 }

fn parent(node: usize) -> usize {
    debug_assert_ne!(node, 0);
    (node - 1) / 2
}

fn sibling(node: usize) -> usize {
    debug_assert_ne!(node, 0);
    if node & 1 == 1 { node + 1 } else { node - 1 }
}

fn block_matches_root(block: &FrameBlock, root: Root) -> bool {
    if block.order > root.order {
        return false;
    }

    let depth = root.order - block.order;
    let first = first_node_at_depth(depth);
    let nodes_at_depth = 1usize << depth;
    if block.node < first || block.node - first >= nodes_at_depth {
        return false;
    }

    let position = block.node - first;
    root.start.checked_add(position * block.frame_count()) == Some(block.start)
}
