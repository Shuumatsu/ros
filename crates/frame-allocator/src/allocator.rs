//! Bitmap-backed buddy allocation over numeric frame ranges.

use core::num::NonZeroUsize;

use heapless::Vec;

use crate::bitmap::{Bitmap, WORD_BITS};
use crate::range::FrameRange;

const MAX_ROOTS: usize = usize::BITS as usize * 2;

const MAX_TREE_DEPTH: usize = usize::BITS as usize;

/// Exact bitmap storage required to manage a frame range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MetadataLayout {
    bits: usize,
    words: usize,
    roots: usize,
}

impl MetadataLayout {
    pub const fn bits(self) -> usize { self.bits }

    pub const fn words(self) -> usize { self.words }

    pub const fn roots(self) -> usize { self.roots }
}

/// Return the exact caller-owned bitmap storage required for `range`.
pub fn metadata_layout(range: FrameRange) -> Result<MetadataLayout, MetadataError> {
    decompose(range).map(|(_, layout)| layout)
}

fn decompose(range: FrameRange) -> Result<(Vec<Root, MAX_ROOTS>, MetadataLayout), MetadataError> {
    let mut roots = Vec::<Root, MAX_ROOTS>::new();
    let mut bits = 0usize;

    for block in range.roots() {
        let frames = block.frame_count();
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MetadataError {
    #[error("frame metadata size exceeds usize")]
    CapacityOverflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum InitError {
    #[error("{0}")]
    Metadata(#[from] MetadataError),
    #[error("insufficient frame metadata: required {required} words, provided {provided}")]
    InsufficientMetadata { required: usize, provided: usize },
}

/// An owned contiguous allocation returned by [`FrameAllocator`].
///
/// This move-only token prevents safe double deallocation.
#[derive(Debug, Eq, PartialEq)]
pub struct FrameBlock {
    start: usize,
    requested_frames: usize,
    order: usize,
    root_index: usize,
    node: usize,
}

impl FrameBlock {
    pub const fn start_frame(&self) -> usize { self.start }

    pub const fn requested_frames(&self) -> usize { self.requested_frames }

    /// Number of frames actually reserved after buddy rounding.
    pub const fn frame_count(&self) -> usize { 1usize << self.order }

    /// Buddy order required by [`FrameAllocator::deallocate_at`].
    pub const fn order(&self) -> usize { self.order }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum DeallocationError {
    #[error("frame block does not belong to this allocator")]
    ForeignBlock,
    #[error("frame block is already free")]
    AlreadyFree,
    #[error("frame {start} does not start an aligned block of order {order}")]
    UnalignedFrame { start: usize, order: usize },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ReserveError {
    #[error("reserved range {start}..{end} is not inside the managed range")]
    OutOfRange { start: usize, end: usize },
    #[error("frame {frame} is already allocated and cannot be reserved")]
    AlreadyAllocated { frame: usize },
}

#[derive(Clone, Copy, Debug)]
struct Root {
    start: usize,
    order: usize,
    bit_offset: usize,
}

/// A buddy allocator over numeric frame identifiers.
///
/// Bookkeeping uses a caller-owned bitmap. Managed frames are never
/// dereferenced or cleared. External synchronization is required for sharing.
pub struct FrameAllocator<'a> {
    range: FrameRange,
    bitmap: Bitmap<'a>,
    roots: Vec<Root, MAX_ROOTS>,
    max_order: usize,
    free_frames: usize,
}

impl<'a> FrameAllocator<'a> {
    /// Manage every frame in `range`.
    ///
    /// The required prefix of `metadata` is cleared and retained for the
    /// allocator's lifetime; additional words are ignored.
    ///
    /// # Safety
    ///
    /// The caller must exclusively own every frame in `range` for the
    /// allocator's lifetime or until each frame is deliberately transferred.
    ///
    /// If `metadata` lies within `range`, reserve all frames backing it before
    /// the first allocation so they are never vended.
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

    pub const fn range(&self) -> FrameRange { self.range }

    pub const fn total_frames(&self) -> usize { self.range.len() }

    pub const fn free_frames(&self) -> usize { self.free_frames }

    /// Frames unavailable for allocation, including buddy rounding.
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

    /// # Safety
    ///
    /// `block` must come from this allocator, remain allocated, and have no
    /// live mapping, pointer, DMA operation, or other owner.
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

    /// Deallocate by the block's first frame and original allocation order.
    ///
    /// Prefer [`deallocate`](Self::deallocate) when the token is available.
    ///
    /// # Safety
    ///
    /// No live owner may access the block. `start` and `order` must exactly
    /// match the original allocation; the allocator cannot validate the order.
    /// A larger order may free live frames and a smaller order leaks capacity.
    ///
    /// Detectable duplicate deallocations return [`DeallocationError::AlreadyFree`].
    pub unsafe fn deallocate_at(
        &mut self,
        start: usize,
        order: usize,
    ) -> Result<(), DeallocationError> {
        let block = self.block_at(start, order)?;
        // SAFETY: `block_at` reconstructs the token under this function's contract.
        unsafe { self.deallocate(block) }
    }

    /// Withhold every frame in `range` from allocation.
    ///
    /// Reserved frames may later be reclaimed with
    /// [`deallocate_at`](Self::deallocate_at) at order zero.
    ///
    /// # Errors
    ///
    /// If a frame was already allocated, previously reserved frames remain
    /// reserved; partial changes are not unwound.
    pub fn reserve(&mut self, range: FrameRange) -> Result<(), ReserveError> {
        if range.start() < self.range.start() || range.end() > self.range.end() {
            return Err(ReserveError::OutOfRange { start: range.start(), end: range.end() });
        }
        for frame in range.start()..range.end() {
            self.claim(frame)?;
        }
        Ok(())
    }

    fn claim(&mut self, start: usize) -> Result<(), ReserveError> {
        let block = self
            .block_at(start, 0)
            .map_err(|_| ReserveError::OutOfRange { start, end: start + 1 })?;
        let root = self.roots[block.root_index];

        let mut path: Vec<usize, MAX_TREE_DEPTH> = Vec::new();
        let mut ancestor = block.node;
        while !self.bitmap.get(root.bit_offset + ancestor) {
            if ancestor == 0 {
                return Err(ReserveError::AlreadyAllocated { frame: start });
            }
            path.push(ancestor).expect("buddy tree deeper than MAX_TREE_DEPTH");
            ancestor = parent(ancestor);
        }

        self.bitmap.clear(root.bit_offset + ancestor);
        for &node in path.iter().rev() {
            self.bitmap.set(root.bit_offset + sibling(node));
        }
        self.free_frames -= block.frame_count();
        Ok(())
    }

    fn block_at(&self, start: usize, order: usize) -> Result<FrameBlock, DeallocationError> {
        let (root_index, root) = self
            .roots
            .iter()
            .copied()
            .enumerate()
            .find(|(_, root)| start >= root.start && start - root.start < (1usize << root.order))
            .ok_or(DeallocationError::ForeignBlock)?;

        if order > root.order {
            return Err(DeallocationError::ForeignBlock);
        }

        let frame_count = 1usize << order;
        let offset = start - root.start;
        if offset % frame_count != 0 {
            return Err(DeallocationError::UnalignedFrame { start, order });
        }

        let depth = root.order - order;
        let node = first_node_at_depth(depth) + offset / frame_count;

        Ok(FrameBlock { start, requested_frames: frame_count, order, root_index, node })
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
