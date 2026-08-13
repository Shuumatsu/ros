//! The kernel heap: this crate's `#[global_allocator]`.
//!
//! A buddy allocator over frames from [`super::frame`], reached through the direct map —
//! so it is a customer of the frame allocator, not a peer, and growing needs no
//! page-table work. Bookkeeping that is not page-shaped belongs here; anything
//! page-sized comes from [`super::frame`] directly.
//!
//! [`INITIAL_SIZE`] is where it starts, not what it is: an allocation that cannot be
//! served takes more frames and retries. [`MAX_SIZE`] is the backstop — a runaway leak
//! dies here, with statistics, rather than draining the pool until a page table is
//! refused a frame.

mod self_test;

use core::alloc::{GlobalAlloc, Layout};
use core::ptr::{self, NonNull};

use buddy_system_allocator::Heap;

use paging::MemoryAddr;
use paging::VirtualAddr;
use paging::sv39::PAGE_SIZE;

use crate::memory::{frame, phys_to_virt};
use crate::sync::IrqMutex;
use crate::utils::ByteSize;

pub use self_test::run as self_test;

/// Buddy order: blocks up to 2^(ORDER-1) bytes, i.e. 2 GiB.
const ORDER: usize = 32;

/// Bytes the heap is given at [`init`].
const INITIAL_SIZE: usize = 8 * 1024 * 1024;

/// Smallest amount added when the heap runs dry. Frames come from the pool a run at a
/// time, so a smaller step would only scatter the heap across the pool.
const GROW_STEP: usize = 2 * 1024 * 1024;

/// Ceiling on the whole heap; see the module docs.
const MAX_SIZE: usize = 64 * 1024 * 1024;

#[global_allocator]
static HEAP: KernelHeap = KernelHeap(IrqMutex::new(Heap::new()));

/// The global allocator: a buddy heap behind a lock that masks interrupts.
///
/// `buddy_system_allocator`'s own `LockedHeap` is deliberately unused — its lock is a
/// bare spin lock, and a `#[global_allocator]` is the one lock guaranteed to be reachable
/// from an interrupt handler. See [`IrqMutex`].
struct KernelHeap(IrqMutex<Heap<ORDER>>);

impl KernelHeap {
    /// Take `pages` frames from the pool and give them to the buddy allocator, returning
    /// where they landed and how many bytes arrived.
    ///
    /// The one place frames become heap; [`init`] and [`grow`](Self::grow) differ only in
    /// how they react to failure. The frame allocation happens with the heap lock
    /// *released*, so the heap lock is never held outside the frame lock.
    fn add_frames(&self, pages: usize) -> Option<(VirtualAddr, usize)> {
        let frames = frame::alloc_contiguous(pages)?;
        // What the pool gave us, not what was asked for; the difference would strand.
        let len = frames.len();
        let start = phys_to_virt(frames.leak());
        // SAFETY: pool frames now owned by the heap for good, mapped read-write through
        // the direct map, reached at no other address, and never released — which is what
        // lets the heap keep its free lists inside them.
        self.0.with(|heap| unsafe { heap.add_to_heap(start.bits(), start.bits() + len) });
        Some((start, len))
    }

    /// Widen the heap by at least `at_least` bytes, or say why not.
    fn grow(&self, at_least: usize) -> bool {
        let total = self.0.with(|heap| heap.stats_total_bytes());
        let step = at_least.max(GROW_STEP).next_multiple_of(PAGE_SIZE);
        if total + step > MAX_SIZE {
            println!(
                "[memory] kernel heap refusing to grow past its {} ceiling ({} in use)",
                ByteSize(MAX_SIZE),
                ByteSize(total)
            );
            return false;
        }
        if self.add_frames(step / PAGE_SIZE).is_none() {
            println!("[memory] kernel heap cannot grow: the frame pool is exhausted");
            return false;
        }
        true
    }
}

// SAFETY: `alloc` returns either null or a block the buddy allocator vended and has not
// vended again; `dealloc` returns a block to the same allocator under the same lock.
unsafe impl GlobalAlloc for KernelHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if let Some(block) = self.0.with(|heap| heap.alloc(layout).ok()) {
            return block.as_ptr();
        }
        // Dry. `grow` sizes its step to cover this request, so one retry is enough: a
        // second failure is out-of-memory, not a too-small step.
        if self.grow(layout.size()) {
            if let Some(block) = self.0.with(|heap| heap.alloc(layout).ok()) {
                return block.as_ptr();
            }
        }
        ptr::null_mut()
    }

    unsafe fn dealloc(&self, block: *mut u8, layout: Layout) {
        let Some(block) = NonNull::new(block) else {
            return;
        };
        // SAFETY: forwarded from the trait's contract — `block` came from `alloc` with
        // this `layout` and is not freed twice.
        self.0.with(|heap| unsafe { heap.dealloc(block, layout) });
    }
}

/// Heap occupancy, in bytes.
#[derive(Clone, Copy, Debug)]
pub struct Stats {
    /// Bytes handed out, after rounding each request up to a buddy block.
    pub used: usize,
    /// Bytes the heap has been given, including what is free.
    pub total: usize,
}

/// What the heap is holding right now.
pub fn stats() -> Stats {
    HEAP.0.with(|heap| Stats { used: heap.stats_alloc_actual(), total: heap.stats_total_bytes() })
}

/// Give the heap its first frames. Call once, on the boot hart, after
/// [`super::frame::init`].
///
/// # Panics
///
/// If the pool cannot produce [`INITIAL_SIZE`] contiguous bytes. Nothing to fall back
/// to: the kernel page table's region list is a `Vec`.
pub fn init() {
    let (start, len) =
        HEAP.add_frames(INITIAL_SIZE / PAGE_SIZE).expect("no contiguous RAM for the kernel heap");
    println!(
        "[memory] heap:   {:#x}..{:#x} ({}, virtual; grows by {} up to {})",
        start,
        start.add(len),
        ByteSize(len),
        ByteSize(GROW_STEP),
        ByteSize(MAX_SIZE)
    );
}

/// Nothing can be done about a failed kernel allocation: no process to kill, no caller
/// that checked. `alloc` has already tried to grow, so a `total` well below [`MAX_SIZE`]
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
