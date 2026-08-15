//! Physical frame allocator: an allocation-free buddy allocator over the RAM above the
//! kernel image, vending page-aligned frames for the heap, page tables, user pages and DMA
//! buffers. [`reserve`] holds what must never be vended.
//!
//! It comes up **first** because it needs no heap: its metadata is a bitmap kept in the
//! pool itself and withheld from the frames vended, so the bookkeeping cannot be allocated
//! away. Where in the pool is `reserve`'s decision, because writing the bitmap precedes
//! every reservation and so cannot assume the front is free. The heap is then carved out
//! of the pool.
//!
//! Frames come out **zeroed** — page tables assume clean PTEs, and a previous owner's
//! bytes must not leak into a fresh mapping. The allocator core never touches frame
//! contents; zeroing is this module's policy, as is reaching them through the direct map.

mod reserve;
mod self_test;

use core::num::NonZeroUsize;

use frame_allocator::{FrameAllocator, FrameBlock, FrameRange, metadata_layout};
use spin::Once;

use paging::MemoryAddr;
use paging::sv39::{PAGE_SIZE, PhysicalAddr};

use crate::memory::machine::PhysRange;
use crate::memory::phys_to_virt;
use crate::sync::IrqMutex;
use crate::utils::ByteSize;

pub use self_test::run as self_test;

/// The one global physical frame allocator. `None` until [`init`].
///
/// An [`IrqMutex`], not a bare spin lock: a fault handler allocating a frame on a hart
/// already inside this lock would deadlock against itself.
static FRAME_ALLOCATOR: IrqMutex<Option<FrameAllocator<'static>>> = IrqMutex::new(None);

/// Frames this module took at [`init`] — one value, so its two ends cannot be read at
/// different times or disagree.
static OWNED: Once<FrameRange> = Once::new();

/// The physical span this module owns, which [`crate::memory::kernel_table`] maps exactly.
///
/// The same range the allocator manages: the metadata bitmap is inside the pool and
/// withheld from it like any other reservation, so there is no second extent to keep in
/// step with this one.
pub fn owned_range() -> (PhysicalAddr, PhysicalAddr) {
    let owned = OWNED.get().expect("frame::owned_range queried before frame::init");
    (PhysicalAddr::from_ppn(owned.start()), PhysicalAddr::from_ppn(owned.end()))
}

/// Pool occupancy, in frames.
#[derive(Clone, Copy, Debug)]
pub struct Stats {
    /// Frames the allocator manages, bitmap excluded.
    pub total: usize,
    /// Frames currently available.
    pub free: usize,
}

impl Stats {
    /// Frames handed out or withheld, including buddy rounding.
    pub fn used(&self) -> usize { self.total - self.free }
}

/// What the pool is holding right now, or `None` before [`init`].
pub fn stats() -> Option<Stats> {
    FRAME_ALLOCATOR.with(|slot| {
        slot.as_ref().map(|allocator| Stats {
            total: allocator.total_frames(),
            free: allocator.free_frames(),
        })
    })
}

/// A physical frame allocation handed out by [`alloc()`] / [`alloc_contiguous`].
///
/// Move-only, so [`free`] consumes it and no allocation can be freed twice in safe code.
/// There is no `Drop` — releasing needs the allocator lock — so a dropped token silently
/// leaks; anything permanent says so with [`leak`](Self::leak).
#[must_use = "a dropped Frames leaks its frames; call leak() to say that is intended"]
pub struct Frames(FrameBlock);

impl Frames {
    /// Base physical address of the run.
    pub fn base(&self) -> PhysicalAddr { PhysicalAddr::from_ppn(self.0.start_frame()) }

    /// Bytes in the run, **after** buddy rounding — not what was asked for. A caller
    /// handing the memory on must pass this along or strand the difference.
    pub fn len(&self) -> usize { self.0.frame_count() * PAGE_SIZE }

    /// Give up the token and keep the frames for good, yielding the base address.
    ///
    /// For the heap, a hart's stack, a page-table root. The frames stay out of the pool
    /// either way; this says it was meant.
    pub fn leak(self) -> PhysicalAddr { self.base() }
}

/// Bring the allocator up over free physical RAM `[free_start, ram_end)`, withholding
/// `foreign` and its own metadata.
///
/// RAM beyond the direct map's window is dropped (loudly), because its frames would have
/// no high-half alias.
///
/// # Panics
///
/// If the pool has already been published. `OWNED`'s `Once` cannot express that on its
/// own: it would silently keep the first range while [`FRAME_ALLOCATOR`] took the second
/// allocator — two answers to which frames the kernel owns — and the bitmap below is
/// taken as a `&'static mut`, which a second call would alias.
pub fn init(free_start: PhysicalAddr, ram_end: PhysicalAddr, foreign: &[PhysRange]) {
    assert!(OWNED.get().is_none(), "frame::init called twice; the pool is already published");

    let direct_map_end = crate::memory::direct_map::DIRECT_MAP_END;
    assert!(
        free_start < direct_map_end,
        "kernel image top {free_start:#x} lies outside the direct map's window, which ends at \
         {direct_map_end:#x}"
    );
    let usable_end = ram_end.min(direct_map_end);
    if ram_end > direct_map_end {
        println!(
            "[memory] WARNING: {} of RAM above the direct map's {:#x} window is unmanaged",
            ByteSize(ram_end.sub_addr(direct_map_end)),
            direct_map_end
        );
    }

    let start_ppn = free_start.align_up(PAGE_SIZE).ppn();
    let end_ppn = usable_end.align_down(PAGE_SIZE).ppn();
    let pool = FrameRange::new(start_ppn, end_ppn).expect("free RAM range empty after alignment");

    let layout = metadata_layout(pool).expect("frame metadata size exceeds usize");
    let bitmap_words = layout.words();
    let bitmap_frames = (bitmap_words * core::mem::size_of::<usize>()).div_ceil(PAGE_SIZE);

    // Where, not just how big. Building the allocator zeroes these frames, and that write
    // happens before a single reservation exists to protect anything from it, so the
    // choice has to respect `foreign` on its own.
    let metadata = reserve::place_metadata(pool, bitmap_frames, foreign);
    let bitmap_va = phys_to_virt(PhysicalAddr::from_ppn(metadata.start()));
    // SAFETY: the bitmap frames are page-aligned, mapped, withheld from the pool below
    // before anything can be vended, and live as long as the kernel, so this `'static mut`
    // borrow is sound and exclusive.
    let bitmap: &'static mut [usize] =
        unsafe { core::slice::from_raw_parts_mut(bitmap_va.as_mut_ptr(), bitmap_words) };

    // SAFETY: `pool` covers RAM strictly above the kernel image — memory nothing else
    // owns — and the boot table maps every frame in it.
    let mut allocator = unsafe {
        FrameAllocator::new(pool, bitmap).expect("frame allocator initialization failed")
    };
    // Before publication, so nothing can be vended out of a carve-out in between.
    reserve::everything_foreign(&mut allocator, pool, metadata, foreign);
    FRAME_ALLOCATOR.with(|slot| *slot = Some(allocator));

    OWNED.call_once(|| pool);
}

/// Print what the kernel owns, what is left, and what was withheld.
pub fn report() {
    let (start, end) = owned_range();
    println!("[memory] frames: {start:#x}..{end:#x} ({}, physical)", ByteSize(end.sub_addr(start)));
    let stats = stats().expect("frame::report called before frame::init");
    println!(
        "[memory]   {} frames, {} free ({}), {} in use",
        stats.total,
        stats.free,
        ByteSize(stats.free * PAGE_SIZE),
        ByteSize(stats.used() * PAGE_SIZE)
    );
    reserve::report();
}

/// Allocate one zeroed physical frame, or `None` if the pool is exhausted.
pub fn alloc() -> Option<Frames> { alloc_contiguous(1) }

/// Allocate `count` physically contiguous zeroed frames, returning the run
/// (`count` is rounded up to a power of two internally, a buddy property, so the
/// base is aligned to the run's size — see [`Frames::len`]).
pub fn alloc_contiguous(count: usize) -> Option<Frames> {
    let count = NonZeroUsize::new(count)?;
    // The lock is released before the zeroing below, which for a large run is far
    // more work than the allocation itself.
    let block = FRAME_ALLOCATOR.with(|slot| slot.as_mut()?.allocate(count))?;
    let frames = Frames(block);
    let base_va = phys_to_virt(frames.base());
    // SAFETY: the allocator just gave us exclusive ownership of these frames,
    // which are mapped writable through the high half.
    unsafe { core::ptr::write_bytes(base_va.as_mut_ptr::<u8>(), 0, frames.len()) };
    Some(frames)
}

/// Return a run to the pool.
///
/// # Safety
/// The frames must no longer be referenced by any live mapping, pointer, or DMA
/// operation.
pub unsafe fn free(frames: Frames) {
    FRAME_ALLOCATOR.with(|slot| {
        let allocator = slot.as_mut().expect("frame allocator used before init");
        // SAFETY: forwarded from this function's contract; `frames` is a move-only
        // token minted by this allocator, so it is neither foreign nor double-freed.
        unsafe { allocator.deallocate(frames.0).expect("frame deallocation failed") };
    });
}

/// Release a single frame identified only by its address.
///
/// The counterpart to [`free`] for a frame whose [`Frames`] token is gone because
/// something else became its record of ownership — in practice a page-table entry.
///
/// # Safety
///
/// The frame must have come from [`alloc()`], i.e. been allocated *singly*: this
/// frees one frame, so calling it on the base of an [`alloc_contiguous`] run
/// leaks the rest. It must no longer be reachable through any live mapping,
/// pointer or DMA operation, and must not already have been freed. A double free
/// is detected rather than silently corrupting the pool, but do not rely on that.
pub unsafe fn free_at(frame: PhysicalAddr) {
    assert!(
        frame.is_aligned(PAGE_SIZE),
        "frame {frame:?} is not page aligned; its page number would be silently rounded down"
    );
    FRAME_ALLOCATOR.with(|slot| {
        let allocator = slot.as_mut().expect("frame allocator used before init");
        // SAFETY: forwarded from this function's contract. Order 0 because `alloc`
        // vends exactly one frame, which is what this function documents accepting.
        unsafe { allocator.deallocate_at(frame.ppn(), 0).expect("frame deallocation failed") };
    });
}
