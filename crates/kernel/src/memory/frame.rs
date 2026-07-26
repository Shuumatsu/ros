//! Physical frame allocator.
//!
//! An allocation-free buddy allocator (`frame_allocator::FrameAllocator`) over
//! the physical RAM that lies *above* the kernel image — the range
//! `[free_start, ram_end)` handed to [`init`] by [`crate::memory::init`]. It
//! vends page-aligned physical frames, single ([`alloc`]) or physically
//! contiguous ([`alloc_contiguous`]), for the kernel heap, page tables, user
//! pages, and DMA buffers.
//!
//! # No heap dependency — this comes up FIRST
//! Unlike a free-list allocator, this keeps its metadata in a caller-supplied
//! bitmap rather than on the heap, so it has **no** dependency on the heap and
//! `memory::init` brings it up *before* the heap — then carves the heap's
//! backing frames out of it. The bitmap itself is reserved from the front of the
//! managed range (see [`init`]) and excluded from the frames it hands out, so
//! the allocator's own metadata can never be allocated away.
//!
//! # Frames come out zeroed
//! Every frame is zeroed before it is handed out — page-table pages assume clean
//! PTEs, and we must never leak a previous owner's bytes into a fresh mapping.
//! The allocator core never touches frame contents; zeroing is our policy here.
//!
//! # Addressing
//! `boot.S` maps RAM both identity and high-half. Kernel-side accesses in this
//! module (the bitmap, zeroing) go through the high-half mapping via
//! [`crate::memory::phys_to_virt`] — the kernel's durable home, which survives
//! the eventual removal of the boot identity map.

use core::num::NonZeroUsize;

use frame_allocator::{FrameAllocator, FrameBlock, FrameRange, metadata_layout};
use spin::Mutex;

use paging::MemoryAddr;
use paging::sv39::{PAGE_SIZE, PhysicalAddr};

use crate::memory::phys_to_virt;

/// The one global physical frame allocator. `None` until [`init`] feeds it a RAM
/// range and the `'static` bitmap reserved from that same RAM.
static FRAME_ALLOCATOR: Mutex<Option<FrameAllocator<'static>>> = Mutex::new(None);

/// A physical frame allocation handed out by [`alloc`] / [`alloc_contiguous`].
///
/// Return it to [`free`] to release. The token is deliberately move-only:
/// freeing consumes it, so the same allocation cannot be freed twice in safe
/// code. Dropping it *without* freeing leaks the frames (there is no `Drop` —
/// releasing needs the allocator lock); [`crate::memory::init`] relies on that
/// to pin the permanent kernel heap.
pub struct Frames(FrameBlock);

impl Frames {
    /// Base physical address of the run.
    pub fn base(&self) -> PhysicalAddr {
        PhysicalAddr::from_ppn(self.0.start_frame())
    }
}

/// Bring the allocator up over free physical RAM `[free_start, ram_end)`.
///
/// The bitmap is reserved from the front of the range and excluded from the
/// managed frames. RAM above the 1 GiB window `boot.S` maps is dropped (loudly),
/// because its frames would not be addressable through either boot mapping.
pub fn init(free_start: usize, ram_end: usize) {
    let ram_base = crate::device_tree::ram_base().expect("device tree RAM base not discovered");
    // boot.S maps a single 1 GiB RAM gigapage; frames past it are unmapped and
    // must not be handed out. No silent truncation — say what we drop.
    let window_end = ram_base + (1usize << 30);
    let usable_end = ram_end.min(window_end);
    if ram_end > window_end {
        println!(
            "[memory] WARNING: {} MiB of RAM above the 1 GiB boot window is unmanaged",
            (ram_end - window_end) / 1024 / 1024
        );
    }

    let start_ppn = PhysicalAddr::new(free_start).align_up(PAGE_SIZE).ppn();
    let end_ppn = PhysicalAddr::new(usable_end).align_down(PAGE_SIZE).ppn();
    let full = FrameRange::new(start_ppn, end_ppn).expect("free RAM range empty after alignment");

    // Size the bitmap for the whole range, then reserve whole frames for it at
    // the front. Sizing for `full` (a superset) always covers the managed range.
    let layout = metadata_layout(full).expect("frame metadata size exceeds usize");
    let bitmap_words = layout.words();
    let bitmap_frames = (bitmap_words * core::mem::size_of::<usize>()).div_ceil(PAGE_SIZE);

    let managed = FrameRange::new(start_ppn + bitmap_frames, end_ppn)
        .expect("no RAM left to manage after reserving the frame bitmap");

    // The bitmap occupies the reserved frames at the front, reached high-half.
    let bitmap_va = phys_to_virt(PhysicalAddr::from_ppn(start_ppn).bits());
    // SAFETY: `[start_ppn, start_ppn + bitmap_frames)` is page-aligned, sits in
    // identity+high-half-mapped RAM, is excluded from `managed`, and lives as
    // long as the kernel (physical RAM), so the `'static mut` borrow is sound
    // and exclusive.
    let bitmap: &'static mut [usize] =
        unsafe { core::slice::from_raw_parts_mut(bitmap_va as *mut usize, bitmap_words) };

    // SAFETY: `managed` covers RAM strictly above the kernel image and the
    // bitmap — memory nothing else owns — and boot.S maps every frame in it.
    // TODO: also reserve the device-tree blob's region; today's range may span
    // it (pre-existing: the old buddy allocator had the same gap).
    let allocator = unsafe {
        FrameAllocator::new(managed, bitmap).expect("frame allocator initialization failed")
    };
    *FRAME_ALLOCATOR.lock() = Some(allocator);
}

/// Allocate one zeroed physical frame, or `None` if the pool is exhausted.
pub fn alloc() -> Option<Frames> {
    alloc_contiguous(1)
}

/// Allocate `count` physically contiguous zeroed frames, returning the run
/// (`count` is rounded up to a power of two internally, a buddy property, so the
/// base is aligned to the run's size).
pub fn alloc_contiguous(count: usize) -> Option<Frames> {
    let count = NonZeroUsize::new(count)?;
    let block = FRAME_ALLOCATOR.lock().as_mut()?.allocate(count)?;
    let base_va = phys_to_virt(PhysicalAddr::from_ppn(block.start_frame()).bits());
    // SAFETY: the allocator just gave us exclusive ownership of these frames,
    // which are mapped writable through the high half.
    unsafe { core::ptr::write_bytes(base_va as *mut u8, 0, block.frame_count() * PAGE_SIZE) };
    Some(Frames(block))
}

/// Return a run to the pool.
///
/// # Safety
/// The frames must no longer be referenced by any live mapping, pointer, or DMA
/// operation.
pub unsafe fn free(frames: Frames) {
    let mut guard = FRAME_ALLOCATOR.lock();
    let allocator = guard.as_mut().expect("frame allocator used before init");
    // SAFETY: forwarded from this function's contract; `frames` is a move-only
    // token minted by this allocator, so it is neither foreign nor double-freed.
    unsafe { allocator.deallocate(frames.0).expect("frame deallocation failed") };
}

/// Smoke-test the allocator immediately after init. A broken frame allocator
/// corrupts page tables and everything mapped through them, so we panic here,
/// loudly and early, rather than limp on.
pub fn self_test() {
    // (1) A freshly allocated frame is page aligned and zeroed.
    let a = alloc().expect("frame self-test: pool empty on first alloc");
    let a_base = a.base();
    assert!(a_base.is_aligned(PAGE_SIZE), "frame {a_base:?} is not page aligned");
    let a_va = phys_to_virt(a_base.bits());
    let first_byte = unsafe { core::ptr::read_volatile(a_va as *const u8) };
    assert_eq!(first_byte, 0u8, "frame {a_base:?} was not zeroed on alloc");

    // (2) A second frame is distinct.
    let b = alloc().expect("frame self-test: pool empty on second alloc");
    assert!(a_base != b.base(), "frame allocator handed out {a_base:?} twice");

    // (3) Dirty then free `a`; if that frame comes back, it must be re-zeroed —
    //     proving alloc zeroes recycled frames, not just pristine RAM.
    unsafe { core::ptr::write_bytes(a_va as *mut u8, 0xAB, PAGE_SIZE) };
    // SAFETY: `a` has no users in this host-free test.
    unsafe { free(a) };
    let c = alloc().expect("frame self-test: pool empty on realloc");
    if c.base() == a_base {
        let byte = unsafe { core::ptr::read_volatile(phys_to_virt(c.base().bits()) as *const u8) };
        assert_eq!(byte, 0u8, "recycled frame {:?} was not re-zeroed (found {byte:#x})", c.base());
    }

    // (4) A 2-frame contiguous run is aligned to its size.
    let run = alloc_contiguous(2).expect("frame self-test: 2-frame contiguous alloc failed");
    assert!(run.base().is_aligned(2 * PAGE_SIZE), "2-frame run {:?} not 8 KiB aligned", run.base());

    // Release everything still held (freeing `c` covers `a`, which became `c`).
    // SAFETY: each token is unique, from this allocator, and has no users.
    unsafe {
        free(b);
        free(c);
        free(run);
    }

    println!("[memory] frame allocator self-test passed");
}
