//! Physical frame allocator.
//!
//! A buddy-system allocator (`buddy_system_allocator::FrameAllocator`) over the
//! physical RAM that lies *above* the kernel heap — the range
//! `[heap_end .. ram_end)` handed to [`add_range`] by [`crate::memory::init`].
//! It vends page-aligned physical frames, single ([`alloc`]) or physically
//! contiguous ([`alloc_contiguous`]), for page tables, user pages, and DMA
//! buffers.
//!
//! # Ordering
//! This allocator keeps its free lists in a `BTreeSet`, i.e. on the kernel
//! heap, so it **cannot** come up before the heap exists. `memory::init`
//! initializes the heap first, then this. That coupling is the whole reason the
//! heap is a bounded region rather than owning all of RAM: something has to feed
//! the heap before the frame allocator can manage the rest.
//!
//! # Frames come out zeroed
//! Every frame is zeroed before it is handed out — page-table pages assume clean
//! PTEs, and we must never leak a previous owner's bytes into a fresh mapping.
//!
//! # Addressing
//! Frames are identity-accessible: paging is off while this runs at boot, and
//! once it is on the kernel identity-maps `heap_start..ram_end`, so a
//! [`PhysicalAddr`] doubles as a valid pointer in both regimes.

use buddy_system_allocator::FrameAllocator;
use spin::Mutex;

use paging::MemoryAddr;
use paging::sv39::{PAGE_SIZE, PhysicalAddr};

/// Buddy max order: `alloc(count)` serves contiguous runs up to `2^(ORDER-1)`
/// frames — far beyond any real request. The only cost of a generous bound is
/// `ORDER` empty `BTreeSet`s, so this is effectively free.
const ORDER: usize = 32;

/// The one global physical frame allocator. Const-initialized empty; fed real
/// RAM by [`add_range`] during `memory::init`.
static FRAME_ALLOCATOR: Mutex<FrameAllocator<ORDER>> = Mutex::new(FrameAllocator::new());

/// Hand the free physical byte range `[start, end)` to the allocator. Both ends
/// are rounded *inward* to whole frames, so a misaligned bound can never yield a
/// partial frame or spill past real RAM.
pub fn add_range(start: usize, end: usize) {
    let start_ppn = PhysicalAddr::new(start).align_up(PAGE_SIZE).ppn();
    let end_ppn = PhysicalAddr::new(end).align_down(PAGE_SIZE).ppn();
    assert!(
        start_ppn < end_ppn,
        "frame pool range empty after alignment: {start:#x}..{end:#x}"
    );
    FRAME_ALLOCATOR.lock().add_frame(start_ppn, end_ppn);
}

/// Allocate one zeroed physical frame, or `None` if the pool is exhausted.
pub fn alloc() -> Option<PhysicalAddr> {
    alloc_contiguous(1)
}

/// Allocate `count` physically contiguous zeroed frames, returning the base of
/// the run (`count` is rounded up to a power of two internally, a buddy
/// property, so the base is aligned to the run's size).
pub fn alloc_contiguous(count: usize) -> Option<PhysicalAddr> {
    let ppn = FRAME_ALLOCATOR.lock().alloc(count)?;
    let base = PhysicalAddr::from_ppn(ppn);
    // SAFETY: the allocator just gave us exclusive ownership of `count` frames
    // at `base`, within identity-accessible RAM (see module docs), so this is a
    // valid, writable region of `count * PAGE_SIZE` bytes.
    unsafe { core::ptr::write_bytes(base.as_mut_ptr::<u8>(), 0, count * PAGE_SIZE) };
    Some(base)
}

/// Return one frame to the pool.
///
/// # Safety
/// `base` must have come from [`alloc`] and must no longer be referenced by any
/// live mapping.
pub unsafe fn dealloc(base: PhysicalAddr) {
    unsafe { dealloc_contiguous(base, 1) };
}

/// Return a contiguous run to the pool.
///
/// # Safety
/// `(base, count)` must exactly match a prior [`alloc_contiguous`] call ([`alloc`]
/// is `count == 1`); buddy dealloc requires the original count. The frames must
/// no longer be referenced by any live mapping.
pub unsafe fn dealloc_contiguous(base: PhysicalAddr, count: usize) {
    FRAME_ALLOCATOR.lock().dealloc(base.ppn(), count);
}

/// Smoke-test the allocator immediately after init. A broken frame allocator
/// corrupts page tables and everything mapped through them, so we panic here,
/// loudly and early, rather than limp on. Assertions print the offending frame.
pub fn self_test() {
    // (1) A freshly allocated frame is page aligned and zeroed.
    let a = alloc().expect("frame self-test: pool empty on first alloc");
    assert!(a.is_aligned(PAGE_SIZE), "frame {a:?} is not page aligned");
    let first_byte = unsafe { core::ptr::read_volatile(a.as_ptr::<u8>()) };
    assert_eq!(first_byte, 0u8, "frame {a:?} was not zeroed on alloc");

    // (2) A second frame is distinct.
    let b = alloc().expect("frame self-test: pool empty on second alloc");
    assert!(a != b, "frame allocator handed out {a:?} twice");

    // (3) Dirty then free `a`; if that frame comes back, it must be re-zeroed —
    //     proving alloc zeroes recycled frames, not just pristine RAM.
    unsafe { core::ptr::write_bytes(a.as_mut_ptr::<u8>(), 0xAB, PAGE_SIZE) };
    unsafe { dealloc(a) };
    let c = alloc().expect("frame self-test: pool empty on realloc");
    if c == a {
        let byte = unsafe { core::ptr::read_volatile(c.as_ptr::<u8>()) };
        assert_eq!(byte, 0u8, "recycled frame {c:?} was not re-zeroed (found {byte:#x})");
    }

    // (4) A 2-frame contiguous run is aligned to its size.
    let run = alloc_contiguous(2).expect("frame self-test: 2-frame contiguous alloc failed");
    assert!(run.is_aligned(2 * PAGE_SIZE), "2-frame run {run:?} not 8 KiB aligned");

    // Release everything still held (freeing `c` covers `a`, which became `c`).
    unsafe {
        dealloc(b);
        dealloc(c);
        dealloc_contiguous(run, 2);
    }

    println!("[memory] frame allocator self-test passed");
}
