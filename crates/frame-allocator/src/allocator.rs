use core::num::NonZeroUsize;

use heapless::Vec;

use crate::bitmap::{Bitmap, WORD_BITS};
use crate::range::FrameRange;

const MAX_ROOTS: usize = usize::BITS as usize * 2;

/// Deepest a buddy tree can be: a tree over `2^order` frames has depth `order`,
/// and an order is bounded by the width of the frame index.
const MAX_TREE_DEPTH: usize = usize::BITS as usize;

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

    /// Buddy order of the allocation: it spans `1 << order` frames.
    ///
    /// Exposed because it is the one value [`FrameAllocator::deallocate_at`]
    /// cannot recover on its own, so a caller that intends to drop this token and
    /// keep only the address must record it first.
    pub const fn order(&self) -> usize { self.order }
}

/// Error detected while returning an allocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum DeallocationError {
    /// The block was created by an allocator with a different range layout, or
    /// names frames this allocator does not manage.
    #[error("frame block does not belong to this allocator")]
    ForeignBlock,
    /// The block, or an ancestor containing it, is already free.
    #[error("frame block is already free")]
    AlreadyFree,
    /// The frame given to [`FrameAllocator::deallocate_at`] does not begin a
    /// block of the requested order, so no such allocation can ever have existed.
    #[error("frame {start} does not start an aligned block of order {order}")]
    UnalignedFrame {
        /// The rejected first frame.
        start: usize,
        /// The order it was claimed to begin.
        order: usize,
    },
}

/// Failure to withhold frames from the pool.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ReserveError {
    /// The requested range is not entirely inside the managed range.
    #[error("reserved range {start}..{end} is not inside the managed range")]
    OutOfRange {
        /// Inclusive start of the rejected range.
        start: usize,
        /// Exclusive end of the rejected range.
        end: usize,
    },
    /// A frame in the range has already been handed out, so it cannot be
    /// withheld — something is using memory the caller believes is spoken for.
    #[error("frame {frame} is already allocated and cannot be reserved")]
    AlreadyAllocated {
        /// The first frame found to be unavailable.
        frame: usize,
    },
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

    /// Return a block identified by its **first frame and order** instead of by
    /// an owned [`FrameBlock`].
    ///
    /// This exists for exactly one reason: reclaiming a page whose only surviving
    /// handle is a page-table entry. A PTE records a frame number, not a token, so
    /// a pager tearing down an address space has an address and nothing else.
    /// Every other caller should keep the token and use
    /// [`deallocate`](Self::deallocate), which cannot be misused.
    ///
    /// # Safety
    ///
    /// Two contracts. The second is what makes this sharper than
    /// [`deallocate`](Self::deallocate), not merely as sharp:
    ///
    /// 1. As for [`deallocate`](Self::deallocate): no live mapping, pointer, DMA
    ///    operation or other owner may still reach any frame in the block.
    /// 2. `order` must be the order the block was **allocated** with, and `start`
    ///    its first frame. Nothing here can check that, and nothing can: the
    ///    bitmap records the extent of *free* blocks, never of allocated ones. Too
    ///    large an `order` frees frames that are still in use; too small an one
    ///    leaks the remainder. Every other misuse below is reported as an error —
    ///    this one is undetectable, which is the whole reason the token-based
    ///    [`deallocate`](Self::deallocate) is the default.
    ///
    /// Note that double frees *are* caught, as
    /// [`DeallocationError::AlreadyFree`]: the ancestor scan in
    /// [`deallocate`](Self::deallocate) finds either the block's own bit or the
    /// coalesced ancestor that swallowed it.
    pub unsafe fn deallocate_at(
        &mut self,
        start: usize,
        order: usize,
    ) -> Result<(), DeallocationError> {
        let block = self.block_at(start, order)?;
        // SAFETY: forwarded from this function's contract. `block_at` rebuilds the
        // token with the same arithmetic `allocate` used to mint it, so it is
        // structurally indistinguishable from the original.
        unsafe { self.deallocate(block) }
    }

    /// Withhold every frame in `range` from the pool, permanently unless it is
    /// later handed back with [`deallocate_at`](Self::deallocate_at).
    ///
    /// For memory that lies inside the managed range but is not the allocator's to
    /// give: a device-tree blob, an initrd, a firmware carve-out. Such memory is
    /// usually *interior* to RAM, so it cannot be excluded by narrowing the range
    /// at construction, and it cannot be claimed with
    /// [`allocate`](Self::allocate) either — that returns whichever block happens
    /// to be free, never a chosen address.
    ///
    /// Reserved frames are indistinguishable from allocated ones afterwards, which
    /// is what makes reclaiming an initrd later just a `deallocate_at`.
    ///
    /// # Errors
    ///
    /// [`ReserveError::AlreadyAllocated`] if any frame in `range` has already been
    /// vended — reserve before vending. **Not unwound**: frames reserved before the
    /// failing one stay reserved. That is the safe direction (they merely go
    /// unused) and it keeps a partial failure from returning memory the caller has
    /// already decided is not free.
    pub fn reserve(&mut self, range: FrameRange) -> Result<(), ReserveError> {
        if range.start() < self.range.start() || range.end() > self.range.end() {
            return Err(ReserveError::OutOfRange { start: range.start(), end: range.end() });
        }
        // Frame at a time, at order 0. A largest-aligned-block walk would touch
        // fewer nodes, but reservations are boot-time and small, and this is
        // obviously correct where the clever version would need its own argument.
        for frame in range.start()..range.end() {
            self.claim(frame)?;
        }
        Ok(())
    }

    /// Mark the single frame `start` as allocated, splitting whichever free block
    /// currently covers it.
    ///
    /// This is [`allocate`](Self::allocate)'s descent aimed at a *chosen* leaf: the
    /// same "clear the free ancestor, then free each sibling on the way down", but
    /// steered along a recorded path instead of always taking the left child.
    fn claim(&mut self, start: usize) -> Result<(), ReserveError> {
        // Order 0 is aligned to everything, and `reserve` has already checked the
        // range, so the only way this fails is a frame outside every root — which
        // is exactly an out-of-range frame.
        let block = self
            .block_at(start, 0)
            .map_err(|_| ReserveError::OutOfRange { start, end: start + 1 })?;
        let root = self.roots[block.root_index];

        // Climb to the nearest ancestor that is a whole free block, recording the
        // path so the split can retrace it downward. Reaching the root without
        // finding one means the frame is already spoken for.
        let mut path: Vec<usize, MAX_TREE_DEPTH> = Vec::new();
        let mut ancestor = block.node;
        while !self.bitmap.get(root.bit_offset + ancestor) {
            if ancestor == 0 {
                return Err(ReserveError::AlreadyAllocated { frame: start });
            }
            path.push(ancestor).expect("buddy tree deeper than MAX_TREE_DEPTH");
            ancestor = parent(ancestor);
        }

        // That ancestor stops being a free block, and every sibling passed on the
        // way down becomes one — leaving exactly the target leaf allocated.
        self.bitmap.clear(root.bit_offset + ancestor);
        for &node in path.iter().rev() {
            self.bitmap.set(root.bit_offset + sibling(node));
        }
        self.free_frames -= block.frame_count();
        Ok(())
    }

    /// Rebuild the [`FrameBlock`] that [`allocate`](Self::allocate) would have
    /// minted for `1 << order` frames starting at `start`.
    ///
    /// Exact inverse of the position arithmetic that closes
    /// [`allocate`](Self::allocate), and deliberately adjacent to it: the two must
    /// agree on how a node index maps to a frame address, so they should be read
    /// together. Nothing else in the crate may derive a node index.
    fn block_at(&self, start: usize, order: usize) -> Result<FrameBlock, DeallocationError> {
        // Roots tile the managed range, so "inside some root" is exactly "managed".
        let (root_index, root) = self
            .roots
            .iter()
            .copied()
            .enumerate()
            .find(|(_, root)| start >= root.start && start - root.start < (1usize << root.order))
            .ok_or(DeallocationError::ForeignBlock)?;

        // Checked before any `1 << order`, which would otherwise overflow. A block
        // larger than its root would have to span roots; those are never handed out.
        if order > root.order {
            return Err(DeallocationError::ForeignBlock);
        }

        let frame_count = 1usize << order;
        let offset = start - root.start;
        if offset % frame_count != 0 {
            return Err(DeallocationError::UnalignedFrame { start, order });
        }

        // `offset < 1 << root.order` and `frame_count == 1 << order`, so
        // `position < 1 << depth` — the node is always within its depth's span.
        let depth = root.order - order;
        let node = first_node_at_depth(depth) + offset / frame_count;

        Ok(FrameBlock {
            start,
            // Nothing records the caller's original pre-rounding request, so
            // report the block's true extent rather than inventing one.
            requested_frames: frame_count,
            order,
            root_index,
            node,
        })
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
