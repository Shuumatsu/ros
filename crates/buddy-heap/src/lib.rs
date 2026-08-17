//! A buddy heap that grows, over memory somebody else supplies.
//!
//! The allocation policy only — how large the heap starts, by how much it widens, and where
//! it stops. Where the memory comes from is the caller's, and so is the lock: this type is
//! deliberately unsynchronized, like the frame allocator it is usually stacked on.
//!
//! [`GrowableHeap::allocate`] never reaches for more memory itself. When it runs dry it
//! *says how much it needs* and returns, so the caller can take its page allocator's lock
//! with this heap's released. A heap that called out to a frame allocator while holding its
//! own lock would nest the two in one direction and forbid the other forever, which is a
//! large promise to make from inside a `#[global_allocator]`.
//!
//! What that buys, beyond the lock discipline: the growth arithmetic is reachable from a
//! host test. Everything below is exercised in `tests/growth.rs` with plain heap memory,
//! no target and no frames.

#![no_std]

use core::alloc::Layout;
use core::ptr::NonNull;

use buddy_system_allocator::Heap;

/// How far a heap may grow, and in what steps.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    /// Bytes to hand over before the first allocation.
    pub initial: usize,
    /// Smallest amount to add when the heap runs dry. A step much larger than a typical
    /// request is the point: growing costs a page-allocator round trip, and a heap grown a
    /// request at a time scatters itself across the pool.
    pub step: usize,
    /// Ceiling on the total ever added. The backstop: a runaway leak dies here, with
    /// statistics, rather than draining the pool until something else is refused a page.
    pub max: usize,
}

/// Heap occupancy, in bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Stats {
    /// Bytes handed out, after rounding each request up to a buddy block.
    pub used: usize,
    /// Bytes the heap has been given, including what is free.
    pub total: usize,
}

/// What [`GrowableHeap::allocate`] decided.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    /// Served out of what the heap already holds.
    Served(NonNull<u8>),
    /// Dry. Hand over at least this many bytes with
    /// [`add_region`](GrowableHeap::add_region) and allocate again.
    Grow {
        /// Bytes the request needs, at minimum. More is welcome and never wasted.
        at_least: usize,
    },
    /// Dry, and growing by what the request needs would pass [`Limits::max`]. Nothing the
    /// caller does short of freeing memory will change this answer.
    AtCeiling {
        /// Bytes the request would have needed.
        wanted: usize,
    },
}

/// A buddy heap of blocks up to `2^(ORDER-1)` bytes, plus the policy for widening it.
pub struct GrowableHeap<const ORDER: usize> {
    heap: Heap<ORDER>,
    limits: Limits,
}

impl<const ORDER: usize> GrowableHeap<ORDER> {
    /// An empty heap. `const`, so this can be a `static` behind the caller's lock.
    ///
    /// The limits are a placeholder until [`configure`](Self::configure): a ceiling worth
    /// having is usually a fraction of memory the caller cannot know at compile time.
    pub const fn new() -> Self {
        Self { heap: Heap::new(), limits: Limits { initial: 0, step: 0, max: 0 } }
    }

    /// Set the limits this heap will honour, before it is given any memory.
    ///
    /// # Panics
    ///
    /// If the heap already holds memory — limits that change under a live heap are limits
    /// nothing was actually held to — or if they are not usable: a zero step cannot make
    /// progress, and an initial size above the ceiling contradicts it.
    pub fn configure(&mut self, limits: Limits) {
        assert_eq!(
            self.heap.stats_total_bytes(),
            0,
            "heap limits must be set before the heap is given memory"
        );
        assert!(limits.initial > 0, "a heap must start with something");
        assert!(limits.step > 0, "a heap that grows by nothing cannot grow");
        assert!(
            limits.initial <= limits.max,
            "heap ceiling {} is below its initial size {}",
            limits.max,
            limits.initial
        );
        self.limits = limits;
    }

    /// The limits in force.
    pub fn limits(&self) -> Limits { self.limits }

    /// What the heap is holding right now.
    pub fn stats(&self) -> Stats {
        Stats { used: self.heap.stats_alloc_actual(), total: self.heap.stats_total_bytes() }
    }

    /// Give `[start, start + len)` to the heap, for good.
    ///
    /// # Safety
    ///
    /// The range must be readable and writable, exclusively owned by this heap from now on,
    /// reachable at no other address, and never released — the free lists live inside it.
    pub unsafe fn add_region(&mut self, start: usize, len: usize) {
        // SAFETY: forwarded from this function's contract.
        unsafe { self.heap.add_to_heap(start, start + len) };
    }

    /// Serve `layout`, or say what would let it be served.
    pub fn allocate(&mut self, layout: Layout) -> Outcome {
        if let Ok(block) = self.heap.alloc(layout) {
            return Outcome::Served(block);
        }
        let wanted = self.growth_for(layout);
        if self.heap.stats_total_bytes() + wanted > self.limits.max {
            return Outcome::AtCeiling { wanted };
        }
        Outcome::Grow { at_least: wanted }
    }

    /// Return a block to the heap.
    ///
    /// # Safety
    ///
    /// `block` must have come from [`allocate`](Self::allocate) on this heap with this
    /// `layout`, and must not already have been returned.
    pub unsafe fn deallocate(&mut self, block: NonNull<u8>, layout: Layout) {
        // SAFETY: forwarded from this function's contract.
        unsafe { self.heap.dealloc(block, layout) };
    }

    /// Bytes to ask for so that `layout` can be served after they arrive.
    ///
    /// Not `layout.size()`: a buddy serves a request from a power-of-two block no smaller
    /// than its alignment, so a heap handed exactly the requested size can come up dry a
    /// second time. Rounding here is what makes one retry enough — and it pairs with a
    /// page allocator that vends a power-of-two run aligned to its own size, which is
    /// exactly one block of the class this asks for.
    fn growth_for(&self, layout: Layout) -> usize {
        let block = layout.size().next_power_of_two().max(layout.align());
        block.max(self.limits.step)
    }
}

impl<const ORDER: usize> Default for GrowableHeap<ORDER> {
    fn default() -> Self { Self::new() }
}
