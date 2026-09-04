//! Growable kernel global allocator.
//!
//! Heap and frame-allocator locks are never nested during growth.

mod self_test;

use core::alloc::{GlobalAlloc, Layout};
use core::ptr::{self, NonNull};

use buddy_heap::{GrowableHeap, Limits, Outcome, Stats};

use mmu::PAGE_SIZE;
use mmu::{MemoryAddr, VirtualAddr};

use super::direct_map::phys_to_virt;
use super::frame;
use crate::sync::IrqMutex;
use crate::utils::{ByteSize, MIB};

pub use self_test::run as self_test;

/// Supports buddy blocks through 2 GiB.
const ORDER: usize = 32;

const INITIAL_SIZE: usize = 8 * MIB;

const GROW_STEP: usize = 2 * MIB;

const MAX_SIZE: usize = 64 * MIB;

/// Maximum frame-pool share held by the heap.
const MAX_POOL_SHARE: usize = 4;

#[global_allocator]
static HEAP: KernelHeap = KernelHeap(IrqMutex::new(GrowableHeap::new()));

/// A buddy heap whose lock masks interrupts to prevent same-hart reentrant deadlock.
struct KernelHeap(IrqMutex<GrowableHeap<ORDER>>);

impl KernelHeap {
    /// Add frame-backed memory while the heap lock is released.
    fn add_frames(&self, at_least: usize) -> Option<(VirtualAddr, usize)> {
        let frames = frame::alloc_contiguous(at_least.div_ceil(PAGE_SIZE))?;
        let len = frames.bytes();
        let start = phys_to_virt(frames.leak());
        // SAFETY: these writable direct-map frames are permanently and exclusively heap-owned.
        self.0.with(|heap| unsafe { heap.add_region(start.bits(), len) });
        Some((start, len))
    }
}

// SAFETY: `GrowableHeap` returns unique layout-compatible blocks, and both operations use it
// under the same lock.
unsafe impl GlobalAlloc for KernelHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut grown = false;
        loop {
            match self.0.with(|heap| heap.allocate(layout)) {
                Outcome::Served(block) => return block.as_ptr(),
                Outcome::Grow { at_least } => {
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
        // SAFETY: forwarded from `GlobalAlloc::dealloc`'s contract.
        self.0.with(|heap| unsafe { heap.deallocate(block, layout) });
    }
}

pub fn stats() -> Stats { HEAP.0.with(|heap| heap.stats()) }

/// Configure and seed the heap after [`super::frame::init`].
///
/// # Panics
///
/// Panics before frame initialization or if the initial contiguous allocation is unavailable.
pub fn init() {
    let pool = frame::stats().expect("heap::init ran before frame::init").total * PAGE_SIZE;
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
