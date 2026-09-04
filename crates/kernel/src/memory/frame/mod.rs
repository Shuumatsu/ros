//! Zeroing physical frame allocator over the machine's RAM bank.
//!
//! Its metadata lives inside the pool and is withheld before the allocator is published.

mod reserve;
mod self_test;

use core::num::NonZeroUsize;

use frame_allocator::{FrameAllocator, FrameBlock, FrameRange, metadata_layout};

use mmu::PAGE_SIZE;
use mmu::{MemoryAddr, PhysicalAddr};

use super::direct_map;
use super::phys_range::PhysRange;
use crate::sync::IrqMutex;
use crate::utils::ByteSize;

pub use self_test::run as self_test;

/// The interrupt-masking lock prevents same-hart reentrant allocation deadlock.
static FRAME_ALLOCATOR: IrqMutex<Option<FrameAllocator<'static>>> = IrqMutex::new(None);

/// Return the full managed span, including ranges withheld within it.
///
/// # Panics
///
/// Panics before [`init`].
pub fn owned_range() -> PhysRange {
    let range = FRAME_ALLOCATOR
        .with(|slot| slot.as_ref().map(FrameAllocator::range))
        .expect("frame::owned_range queried before frame::init");
    reserve::phys_range("frame pool", range)
}

#[derive(Clone, Copy, Debug)]
pub struct Stats {
    pub total: usize,
    pub free: usize,
}

impl Stats {
    pub fn used(&self) -> usize { self.total - self.free }
}

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
/// Dropping the move-only token leaks the run; call [`free`] or [`leak`](Self::leak) explicitly.
#[must_use = "a dropped Frames leaks its frames; call leak() to say that is intended"]
pub struct Frames(FrameBlock);

impl Frames {
    pub fn base(&self) -> PhysicalAddr { PhysicalAddr::from_ppn(self.0.start_frame()) }

    /// Run size after buddy rounding, which may exceed the requested frame count.
    pub fn bytes(&self) -> usize { self.0.frame_count() * PAGE_SIZE }

    /// Permanently retain the run and return its base.
    pub fn leak(self) -> PhysicalAddr { self.base() }
}

/// Initialize the pool and withhold `image`, `foreign`, and allocator metadata.
///
/// RAM beyond the direct-map window remains unmanaged.
///
/// # Panics
///
/// Panics on repeated initialization or unusable pool geometry.
pub fn init(ram: &PhysRange, image: PhysRange, foreign: &[PhysRange]) {
    assert!(
        FRAME_ALLOCATOR.with(|slot| slot.is_none()),
        "frame::init called twice; the pool is already published"
    );

    let ram_end = ram.end();
    let usable_end = ram_end.min(direct_map::END);
    if ram_end > direct_map::END {
        println!(
            "[memory] WARNING: {} of RAM above the direct map's {:#x} window is unmanaged",
            ByteSize(ram_end.sub_addr(direct_map::END)),
            direct_map::END
        );
    }

    let start_ppn = ram.base.align_up(PAGE_SIZE).ppn();
    let end_ppn = usable_end.align_down(PAGE_SIZE).ppn();
    let pool = FrameRange::new(start_ppn, end_ppn).expect("RAM bank empty after alignment");

    let mut carveouts: heapless::Vec<PhysRange, { reserve::MAX_CARVEOUTS }> = heapless::Vec::new();
    carveouts.push(image).expect("MAX_CARVEOUTS leaves a slot for the kernel image");
    carveouts
        .extend_from_slice(foreign)
        .expect("MachineMemory::check bounds the machine's list at MAX_FOREIGN");

    let layout = metadata_layout(pool).expect("frame metadata size exceeds usize");
    let bitmap_words = layout.words();
    let bitmap_frames = (bitmap_words * core::mem::size_of::<usize>()).div_ceil(PAGE_SIZE);

    // Allocator construction writes the bitmap before reservations exist.
    let metadata = reserve::place_metadata(pool, bitmap_frames, &carveouts);
    let bitmap_va = direct_map::phys_to_virt(PhysicalAddr::from_ppn(metadata.start()));
    // SAFETY: aligned bitmap frames are mapped, exclusive, withheld below, and permanent.
    let bitmap: &'static mut [usize] =
        unsafe { core::slice::from_raw_parts_mut(bitmap_va.as_mut_ptr(), bitmap_words) };

    // SAFETY: the boot table maps `pool`; all pre-owned ranges and the in-pool bitmap are
    // withheld before publication.
    let mut allocator = unsafe {
        FrameAllocator::new(pool, bitmap).expect("frame allocator initialization failed")
    };
    reserve::withhold_all(&mut allocator, pool, metadata, &carveouts);
    FRAME_ALLOCATOR.with(|slot| *slot = Some(allocator));
}

pub fn report() {
    let pool = owned_range();
    println!(
        "[memory] frames: {:#x}..{:#x} ({}, physical)",
        pool.base,
        pool.end(),
        ByteSize(pool.size)
    );
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

/// Allocate contiguous zeroed frames.
///
/// Buddy rounding may increase `count`; the base is aligned to [`Frames::bytes`].
pub fn alloc_contiguous(count: usize) -> Option<Frames> {
    let count = NonZeroUsize::new(count)?;
    let block = FRAME_ALLOCATOR.with(|slot| slot.as_mut()?.allocate(count))?;
    let frames = Frames(block);
    let base_va = direct_map::phys_to_virt(frames.base());
    // SAFETY: the allocated frames are exclusive and writable through the direct map.
    unsafe { core::ptr::write_bytes(base_va.as_mut_ptr::<u8>(), 0, frames.bytes()) };
    Some(frames)
}

/// Return a run to the pool.
///
/// # Safety
///
/// The frames must have no live mapping, pointer, or DMA reference.
pub unsafe fn free(frames: Frames) {
    FRAME_ALLOCATOR.with(|slot| {
        let allocator = slot.as_mut().expect("frame allocator used before init");
        // SAFETY: the token proves allocator provenance and uniqueness; the caller proves quiescence.
        unsafe { allocator.deallocate(frames.0).expect("frame deallocation failed") };
    });
}

/// Release one frame by address.
///
/// # Safety
///
/// The frame must come from [`alloc()`], remain allocated, and have no live mapping, pointer, or
/// DMA reference. Passing part of a contiguous run is invalid.
pub unsafe fn free_at(frame: PhysicalAddr) {
    assert!(
        frame.is_aligned(PAGE_SIZE),
        "frame {frame:?} is not page aligned; its page number would be silently rounded down"
    );
    FRAME_ALLOCATOR.with(|slot| {
        let allocator = slot.as_mut().expect("frame allocator used before init");
        // SAFETY: the caller guarantees a live order-0 allocation with no users.
        unsafe { allocator.deallocate_at(frame.ppn(), 0).expect("frame deallocation failed") };
    });
}
