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
//! `boot.S` maps physical memory both identity and into the kernel's direct map
//! (see [`crate::memory::direct_map`]). Kernel-side accesses in this module (the
//! bitmap, zeroing) go through the direct map via
//! [`crate::memory::phys_to_virt`] — the kernel's durable home, which survives
//! the eventual removal of the boot identity map.

use core::num::NonZeroUsize;
use core::sync::atomic::{AtomicUsize, Ordering};

use frame_allocator::{FrameAllocator, FrameBlock, FrameRange, metadata_layout};
use spin::Mutex;

use paging::MemoryAddr;
use paging::sv39::{FrameSource, PAGE_SIZE, PhysicalAddr};

use crate::memory::phys_to_virt;

/// The one global physical frame allocator. `None` until [`init`] feeds it a RAM
/// range and the `'static` bitmap reserved from that same RAM.
static FRAME_ALLOCATOR: Mutex<Option<FrameAllocator<'static>>> = Mutex::new(None);

/// Physical span this module took at [`init`], bitmap included. Zero until then.
static OWNED_START: AtomicUsize = AtomicUsize::new(0);
static OWNED_END: AtomicUsize = AtomicUsize::new(0);

/// The physical span this module owns, `[start, end)`, **including** the metadata
/// bitmap that [`init`] reserved from the front of it.
///
/// The authoritative answer to "which physical memory belongs to the frame
/// subsystem", and therefore to what a page table must map before the kernel can
/// touch any of it. [`super::kernel_table`] derives its direct map from this
/// rather than re-deriving a RAM extent of its own — the allocator decides what it
/// will hand out, and the table maps exactly that.
///
/// Note this is deliberately *not* the allocator's `range()`, which excludes the
/// bitmap: the bitmap needs mapping too, since this module writes it.
pub fn owned_range() -> (usize, usize) {
    let start = OWNED_START.load(Ordering::Relaxed);
    let end = OWNED_END.load(Ordering::Relaxed);
    assert!(start < end, "frame::owned_range queried before frame::init");
    (start, end)
}

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
/// managed frames. RAM above the window `boot.S` maps is dropped (loudly),
/// because its frames would not be addressable through either boot mapping.
pub fn init(free_start: usize, ram_end: usize) {
    // Frames past the boot mappings are unreachable and must not be handed out.
    // The bound comes from the module that *builds* those mappings — this used to
    // re-derive it as `ram_base + 1 GiB`, an independent re-encoding of boot.S's
    // decision that would have kept clamping at 1 GiB if the window ever grew.
    // No silent truncation either — say what we drop.
    let window_end = crate::memory::direct_map::WINDOW_END;
    // The window is absolute (from PA 0), not RAM-relative, so state the case it
    // cannot serve rather than letting `FrameRange::new` below fail with a
    // confusing "range empty after alignment". Unreachable on any platform that
    // got this far — the kernel itself would be outside the mapping — but the
    // diagnostic is what makes that obvious instead of mysterious.
    assert!(
        free_start < window_end,
        "kernel image top {free_start:#x} lies outside the {window_end:#x} boot mapping window; \
         the direct map does not reach this platform's RAM (see memory::direct_map)"
    );
    let usable_end = ram_end.min(window_end);
    if ram_end > window_end {
        println!(
            "[memory] WARNING: {} MiB of RAM above the {:#x} boot window is unmanaged",
            (ram_end - window_end) / 1024 / 1024,
            window_end
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
    let mut allocator = unsafe {
        FrameAllocator::new(managed, bitmap).expect("frame allocator initialization failed")
    };
    reserve_device_tree(&mut allocator, managed);
    *FRAME_ALLOCATOR.lock() = Some(allocator);

    // Publish the span we took — bitmap included, so `[start_ppn, end_ppn)` rather
    // than `managed`. This is what `kernel_table` maps; see `owned_range`.
    OWNED_START.store(PhysicalAddr::from_ppn(start_ppn).bits(), Ordering::Relaxed);
    OWNED_END.store(PhysicalAddr::from_ppn(end_ppn).bits(), Ordering::Relaxed);
}

/// Withhold the frames the device-tree blob occupies.
///
/// The previous boot stage leaves the blob in ordinary RAM — on QEMU virt at
/// `0x87e00000`, near the top — so it falls squarely inside `managed`. Without
/// this the allocator will happily vend the pages the tree is stored in, and the
/// corruption only shows up if something reads the blob again.
///
/// Rounded outward to whole frames: a partial frame is still a frame that must not
/// be handed out.
fn reserve_device_tree(allocator: &mut FrameAllocator<'static>, managed: FrameRange) {
    let Some((dtb_start, dtb_end)) = crate::device_tree::dtb_range() else {
        panic!("device tree extent unknown; call device_tree::init before memory::init")
    };

    let first = PhysicalAddr::new(dtb_start).align_down(PAGE_SIZE).ppn();
    let last = PhysicalAddr::new(dtb_end).align_up(PAGE_SIZE).ppn();

    // The blob need not be inside the pool: it could sit below the kernel image, or
    // above the mapped window we clamped to. Reserve only the overlap, and say when
    // there is none rather than leaving it ambiguous.
    let first = first.max(managed.start());
    let last = last.min(managed.end());
    let Ok(range) = FrameRange::new(first, last) else {
        println!(
            "[memory] device tree at {dtb_start:#x}..{dtb_end:#x} lies outside the frame pool; \
             nothing to reserve"
        );
        return;
    };

    let free_before = allocator.free_frames();
    allocator
        .reserve(range)
        .unwrap_or_else(|error| panic!("reserving the device tree blob failed: {error}"));
    // A reservation that silently withheld nothing would leave the blob vendable
    // and the corruption would only surface much later, so check the accounting
    // actually moved rather than trusting the call.
    assert_eq!(
        allocator.free_frames(),
        free_before - range.len(),
        "reserving {} device-tree frames did not remove them from the pool",
        range.len()
    );
    println!(
        "[memory] reserved device tree: {:#x}..{:#x} ({} frames)",
        PhysicalAddr::from_ppn(range.start()).bits(),
        PhysicalAddr::from_ppn(range.end()).bits(),
        range.len()
    );
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

/// Release a single frame identified only by its address.
///
/// The counterpart to [`free`] for a frame whose [`Frames`] token is gone because
/// something else became its record of ownership — in practice a page-table entry.
///
/// # Safety
///
/// The frame must have come from [`alloc`], i.e. been allocated *singly*: this
/// frees one frame, so calling it on the base of an [`alloc_contiguous`] run
/// leaks the rest. It must no longer be reachable through any live mapping,
/// pointer or DMA operation, and must not already have been freed. A double free
/// is detected rather than silently corrupting the pool, but do not rely on that.
pub unsafe fn free_at(frame: PhysicalAddr) {
    assert!(
        frame.is_aligned(PAGE_SIZE),
        "frame {frame:?} is not page aligned; its page number would be silently rounded down"
    );
    let mut guard = FRAME_ALLOCATOR.lock();
    let allocator = guard.as_mut().expect("frame allocator used before init");
    // SAFETY: forwarded from this function's contract. Order 0 because `alloc`
    // vends exactly one frame, which is what this function documents accepting.
    unsafe {
        allocator.deallocate_at(frame.ppn(), 0).expect("frame deallocation failed");
    }
}

/// Supplies the frames that intermediate page tables live in.
///
/// # Why the token is dropped
///
/// [`FrameSource::alloc_zeroed`] returns a bare [`PhysicalAddr`] and lets the
/// [`Frames`] token go. That is not an accidental leak, it is the handoff: the
/// moment the frame is installed as a branch PTE, the *page table* becomes its
/// record of ownership. Reclaiming it later means walking to that entry and
/// passing the address it holds to [`free_at`] — which is exactly what
/// [`FrameSource::free`] does below, and why [`free_at`] has to exist at all.
pub struct TableFrames;

// SAFETY: `alloc` returns a page-aligned frame, freshly zeroed (see the module
// docs — zeroing is this module's policy, and a zeroed frame is what makes a new
// table read as "all entries invalid"), owned exclusively by the caller until it
// comes back through `free_at`.
unsafe impl FrameSource for TableFrames {
    fn alloc_zeroed(&mut self) -> Option<PhysicalAddr> {
        alloc().map(|frames| frames.base())
    }

    unsafe fn free(&mut self, frame: PhysicalAddr) {
        // SAFETY: forwarded from the trait's contract, which requires the frame
        // to have come from this source and to be unreachable from any live page
        // table. `alloc_zeroed` only ever vends single frames, satisfying
        // `free_at`'s order-0 requirement.
        unsafe { free_at(frame) };
    }
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
