//! An unsynchronized buddy heap over caller-supplied memory.
//!
//! Allocation reports required growth but never obtains memory itself.

#![no_std]

use core::alloc::Layout;
use core::ptr::NonNull;

use buddy_system_allocator::Heap;

/// Heap growth limits in bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    pub initial: usize,
    /// Minimum growth request.
    pub step: usize,
    /// Maximum total supplied memory.
    pub max: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Stats {
    /// Allocated bytes after buddy rounding.
    pub used: usize,
    /// Total supplied bytes.
    pub total: usize,
}

/// What [`GrowableHeap::allocate`] decided.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    Served(NonNull<u8>),
    /// Supply at least `at_least` bytes and retry.
    Grow {
        at_least: usize,
    },
    /// Required growth would exceed [`Limits::max`].
    AtCeiling {
        wanted: usize,
    },
}

/// A growable buddy heap with blocks up to `2^(ORDER-1)` bytes.
pub struct GrowableHeap<const ORDER: usize> {
    heap: Heap<ORDER>,
    limits: Limits,
}

impl<const ORDER: usize> GrowableHeap<ORDER> {
    /// Create an empty, unconfigured heap.
    pub const fn new() -> Self {
        Self { heap: Heap::new(), limits: Limits { initial: 0, step: 0, max: 0 } }
    }

    /// Configure the heap before adding memory.
    ///
    /// # Panics
    ///
    /// If the heap already has memory, either `initial` or `step` is zero, or
    /// `initial` exceeds `max`.
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

    pub fn limits(&self) -> Limits { self.limits }

    pub fn stats(&self) -> Stats {
        Stats { used: self.heap.stats_alloc_actual(), total: self.heap.stats_total_bytes() }
    }

    /// Permanently add `[start, start + len)` to the heap.
    ///
    /// # Safety
    ///
    /// The range must be readable, writable, uniquely addressed, and exclusively
    /// owned by this heap forever because it stores the free lists.
    pub unsafe fn add_region(&mut self, start: usize, len: usize) {
        // SAFETY: forwarded from this function's contract.
        unsafe { self.heap.add_to_heap(start, start + len) };
    }

    /// Allocate from current memory or report the required growth.
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

    /// # Safety
    ///
    /// `block` must be a live allocation from this heap made with `layout`.
    pub unsafe fn deallocate(&mut self, block: NonNull<u8>, layout: Layout) {
        // SAFETY: forwarded from this function's contract.
        unsafe { self.heap.dealloc(block, layout) };
    }

    /// Required growth, rounded for both buddy size and alignment.
    ///
    /// Supplying an aligned power-of-two region of this size makes one retry
    /// sufficient.
    fn growth_for(&self, layout: Layout) -> usize {
        let block = layout.size().next_power_of_two().max(layout.align());
        block.max(self.limits.step)
    }
}

impl<const ORDER: usize> Default for GrowableHeap<ORDER> {
    fn default() -> Self { Self::new() }
}
