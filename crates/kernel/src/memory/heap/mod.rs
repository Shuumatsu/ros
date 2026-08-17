//! The kernel heap: this crate's `#[global_allocator]`.
//!
//! Three things live here and nothing else: the lock, the frames, and the limits. The
//! allocator itself is [`buddy_heap::GrowableHeap`] — a crate, so its growth arithmetic is
//! exercised on the host rather than only on the way through a boot. It is named
//! `buddy-heap` and not `heap` so that a reader of this file never has to work out whether
//! `heap::` means the crate or the module they are in.
//!
//! Frames come from [`super::frame`] and are reached through the direct map, so this is a
//! customer of the frame allocator rather than a peer and growing needs no page-table work.
//! Bookkeeping that is not page-shaped belongs here; anything page-sized comes from
//! [`super::frame`] directly.
//!
//! The heap asks for memory and this module fetches it, which is what keeps the two locks
//! ordered: the heap's is released before the frame allocator's is taken, never nested.

mod self_test;

use core::alloc::{GlobalAlloc, Layout};
use core::ptr::{self, NonNull};

use buddy_heap::{GrowableHeap, Limits, Outcome, Stats};

use paging::PAGE_SIZE;
use paging::utils::MEGABYTE;
use paging::{MemoryAddr, VirtualAddr};

use super::direct_map::phys_to_virt;
use super::frame;
use crate::sync::IrqMutex;
use crate::utils::ByteSize;

pub use self_test::run as self_test;

/// Buddy order: blocks up to 2^(ORDER-1) bytes, i.e. 2 GiB.
const ORDER: usize = 32;

/// Bytes the heap is given at [`init`].
const INITIAL_SIZE: usize = 8 * MEGABYTE;

/// Smallest amount added when the heap runs dry. Frames come from the pool a run at a
/// time, so a smaller step would only scatter the heap across the pool.
const GROW_STEP: usize = 2 * MEGABYTE;

/// Ceiling on the whole heap, unless the machine is too small to spare it.
const MAX_SIZE: usize = 64 * MEGABYTE;

/// Largest share of the frame pool the heap may ever hold, as a divisor.
///
/// A fixed ceiling is no backstop on a machine whose RAM is smaller than the ceiling: the
/// pool drains first and the page tables are what get refused. So the limit is whichever of
/// the two is tighter, and the boot log says which one won.
const MAX_POOL_SHARE: usize = 4;

#[global_allocator]
static HEAP: KernelHeap = KernelHeap(IrqMutex::new(GrowableHeap::new()));

/// The global allocator: a growable buddy heap behind a lock that masks interrupts.
///
/// The lock is an [`IrqMutex`] rather than a bare spin lock because a `#[global_allocator]`
/// is the one lock guaranteed to be reachable from an interrupt handler — a handler that
/// allocated while ordinary code held a plain spin lock would deadlock against itself.
struct KernelHeap(IrqMutex<GrowableHeap<ORDER>>);

impl KernelHeap {
    /// Take at least `at_least` bytes from the frame pool and give them to the heap,
    /// returning where they landed and how many arrived.
    ///
    /// The one place frames become heap. The frame allocation happens with the heap lock
    /// *released*, which is the whole reason the heap asks instead of fetching: the heap
    /// lock is never held outside the frame lock.
    fn add_frames(&self, at_least: usize) -> Option<(VirtualAddr, usize)> {
        let frames = frame::alloc_contiguous(at_least.div_ceil(PAGE_SIZE))?;
        // What the pool gave us, not what was asked for; the difference would strand.
        let len = frames.bytes();
        let start = phys_to_virt(frames.leak());
        // SAFETY: pool frames now owned by the heap for good, mapped read-write through
        // the direct map, reached at no other address, and never released — which is what
        // lets the heap keep its free lists inside them.
        self.0.with(|heap| unsafe { heap.add_region(start.bits(), len) });
        Some((start, len))
    }
}

// SAFETY: `alloc` returns either null or a block the buddy allocator vended and has not
// vended again; `dealloc` returns a block to the same allocator under the same lock.
unsafe impl GlobalAlloc for KernelHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut grown = false;
        loop {
            match self.0.with(|heap| heap.allocate(layout)) {
                Outcome::Served(block) => return block.as_ptr(),
                Outcome::Grow { at_least } => {
                    // Growing twice for one request would mean the frames arrived and did
                    // not help, which the sizing rules out: the heap asks for the buddy
                    // block this request is served from, and the pool vends a run aligned
                    // to its own size. Bounded anyway — an unbounded retry against a
                    // mistake in either half is an infinite loop inside the allocator.
                    // The `alloc_error` handler reports the numbers.
                    if grown {
                        return ptr::null_mut();
                    }
                    grown = true;
                    if self.add_frames(at_least).is_none() {
                        println!("[memory] kernel heap cannot grow: the frame pool is exhausted");
                        return ptr::null_mut();
                    }
                }
                Outcome::AtCeiling { wanted } => {
                    let Stats { total, .. } = self.0.with(|heap| heap.stats());
                    println!(
                        "[memory] kernel heap refusing to grow by {} past its {} ceiling \
                         ({} given out)",
                        ByteSize(wanted),
                        ByteSize(self.0.with(|heap| heap.limits()).max),
                        ByteSize(total)
                    );
                    return ptr::null_mut();
                }
            }
        }
    }

    unsafe fn dealloc(&self, block: *mut u8, layout: Layout) {
        let Some(block) = NonNull::new(block) else {
            return;
        };
        // SAFETY: forwarded from the trait's contract — `block` came from `alloc` with
        // this `layout` and is not freed twice.
        self.0.with(|heap| unsafe { heap.deallocate(block, layout) });
    }
}

/// What the heap is holding right now.
pub fn stats() -> Stats { HEAP.0.with(|heap| heap.stats()) }

/// Give the heap its limits and its first frames. Call once, on the boot hart, after
/// [`super::frame::init`].
///
/// # Panics
///
/// If the pool cannot produce [`INITIAL_SIZE`] contiguous bytes. Nothing to fall back
/// to: the kernel page table's region list is a `Vec`.
pub fn init() {
    let pool = frame::stats().expect("heap::init ran before frame::init").total * PAGE_SIZE;
    // Whole steps: the heap only ever grows by one, so a remainder is unreachable anyway.
    // Never below the initial size — that has to fit, and a pool too small to hold it fails
    // in `add_frames` below, where the message is about the memory rather than the policy.
    let share = (pool / MAX_POOL_SHARE) / GROW_STEP * GROW_STEP;
    let ceiling = MAX_SIZE.min(share).max(INITIAL_SIZE);
    let limits = Limits { initial: INITIAL_SIZE, step: GROW_STEP, max: ceiling };
    HEAP.0.with(|heap| heap.configure(limits));

    let (start, len) =
        HEAP.add_frames(limits.initial).expect("no contiguous RAM for the kernel heap");
    println!(
        "[memory] heap:   {:#x}..{:#x} ({}, virtual; grows by {} up to {})",
        start,
        start.add(len),
        ByteSize(len),
        ByteSize(limits.step),
        ByteSize(limits.max)
    );
    if ceiling < MAX_SIZE {
        println!(
            "[memory]   ceiling is 1/{MAX_POOL_SHARE} of the {} pool, not the {} default",
            ByteSize(pool),
            ByteSize(MAX_SIZE)
        );
    }
}

/// Nothing can be done about a failed kernel allocation: no process to kill, no caller
/// that checked. `alloc` has already tried to grow, so a `total` well below the ceiling
/// means the *pool* ran out.
#[alloc_error_handler]
fn alloc_error(layout: Layout) -> ! {
    let Stats { used, total } = stats();
    panic!(
        "kernel heap exhausted: {} bytes at {}-byte alignment; {} of {} in use",
        layout.size(),
        layout.align(),
        ByteSize(used),
        ByteSize(total)
    )
}
