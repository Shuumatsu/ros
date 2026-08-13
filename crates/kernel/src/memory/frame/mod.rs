//! Physical frame allocator.
//!
//! An allocation-free buddy allocator (`frame_allocator::FrameAllocator`) over the
//! physical RAM that lies *above* the kernel image — the range `[free_start, ram_end)`
//! handed to [`init`] by [`crate::memory::init`]. It vends page-aligned physical
//! frames, single ([`alloc`]) or physically contiguous ([`alloc_contiguous`]), for the
//! kernel heap, page tables, user pages, and DMA buffers.
//!
//! # No heap dependency — this comes up FIRST
//! Metadata lives in a bitmap reserved from the front of the managed range (see
//! [`init`]) and excluded from the frames handed out, not on the heap. So this comes up
//! *before* the heap and then carves the heap's backing frames out of itself, and the
//! allocator's own metadata can never be allocated away.
//!
//! # Frames come out zeroed
//! Page-table pages assume clean PTEs, and a previous owner's bytes must never leak
//! into a fresh mapping. The allocator core never touches frame contents; zeroing is
//! this module's policy.
//!
//! # Addressing
//! Kernel-side accesses here (the bitmap, zeroing) go through the direct map via
//! [`crate::memory::phys_to_virt`] rather than the boot identity map, which the kernel
//! will eventually drop.

use core::num::NonZeroUsize;
use core::sync::atomic::{AtomicUsize, Ordering};

use frame_allocator::{FrameAllocator, FrameBlock, FrameRange, metadata_layout};
use heapless::{String, Vec};
use spin::Mutex;

use paging::MemoryAddr;
use paging::sv39::{FrameSource, PAGE_SIZE, PhysicalAddr};

use crate::memory::phys_to_virt;
use crate::utils::ByteSize;

/// The one global physical frame allocator. `None` until [`init`] feeds it a RAM
/// range and the `'static` bitmap reserved from that same RAM.
static FRAME_ALLOCATOR: Mutex<Option<FrameAllocator<'static>>> = Mutex::new(None);

/// Physical span this module took at [`init`], bitmap included. Zero until then.
///
/// Bare words because there is no atomic address type; [`owned_range`] is the only
/// reader and it puts the type back on.
static OWNED_START: AtomicUsize = AtomicUsize::new(0);
static OWNED_END: AtomicUsize = AtomicUsize::new(0);

/// The physical span this module owns, `[start, end)`, **including** the metadata
/// bitmap that [`init`] reserved from the front of it.
///
/// [`super::kernel_table`] derives its direct map from this rather than re-deriving a
/// RAM extent of its own: the allocator decides what it will hand out, and the table
/// maps exactly that.
///
/// Deliberately *not* the allocator's `range()`, which excludes the bitmap — the
/// bitmap needs mapping too, since this module writes it.
pub fn owned_range() -> (PhysicalAddr, PhysicalAddr) {
    let start = PhysicalAddr::new(OWNED_START.load(Ordering::Relaxed));
    let end = PhysicalAddr::new(OWNED_END.load(Ordering::Relaxed));
    assert!(start < end, "frame::owned_range queried before frame::init");
    (start, end)
}

/// A physical frame allocation handed out by [`alloc`] / [`alloc_contiguous`].
///
/// Return it to [`free`] to release. Move-only, so freeing consumes it and the same
/// allocation cannot be freed twice in safe code. Dropping it *without* freeing leaks
/// the frames (there is no `Drop` — releasing needs the allocator lock);
/// [`crate::memory::init`] relies on that to pin the permanent kernel heap.
pub struct Frames(FrameBlock);

impl Frames {
    /// Base physical address of the run.
    pub fn base(&self) -> PhysicalAddr { PhysicalAddr::from_ppn(self.0.start_frame()) }
}

/// Bring the allocator up over free physical RAM `[free_start, ram_end)`.
///
/// The bitmap is reserved from the front of the range and excluded from the
/// managed frames. RAM beyond the Sv39 direct-map capacity is dropped (loudly),
/// because its frames would have no high-half alias.
pub fn init(free_start: PhysicalAddr, ram_end: PhysicalAddr) {
    let direct_map_end = crate::memory::direct_map::DIRECT_MAP_END;
    assert!(
        free_start < direct_map_end,
        "kernel image top {free_start:#x} lies outside the Sv39 direct map ending at \
         {direct_map_end:#x}"
    );
    let usable_end = ram_end.min(direct_map_end);
    if ram_end > direct_map_end {
        println!(
            "[memory] WARNING: {} of RAM above the {:#x} Sv39 direct-map limit is unmanaged",
            crate::utils::ByteSize(ram_end.sub_addr(direct_map_end)),
            direct_map_end
        );
    }

    let start_ppn = free_start.align_up(PAGE_SIZE).ppn();
    let end_ppn = usable_end.align_down(PAGE_SIZE).ppn();
    let full = FrameRange::new(start_ppn, end_ppn).expect("free RAM range empty after alignment");

    // Size the bitmap for the whole range, then reserve whole frames for it at
    // the front. Sizing for `full` (a superset) always covers the managed range.
    let layout = metadata_layout(full).expect("frame metadata size exceeds usize");
    let bitmap_words = layout.words();
    let bitmap_frames = (bitmap_words * core::mem::size_of::<usize>()).div_ceil(PAGE_SIZE);

    let managed = FrameRange::new(start_ppn + bitmap_frames, end_ppn)
        .expect("no RAM left to manage after reserving the frame bitmap");

    let bitmap_va = phys_to_virt(PhysicalAddr::from_ppn(start_ppn));
    // SAFETY: `[start_ppn, start_ppn + bitmap_frames)` is page-aligned, sits in
    // identity+high-half-mapped RAM, is excluded from `managed`, and lives as
    // long as the kernel (physical RAM), so the `'static mut` borrow is sound
    // and exclusive.
    let bitmap: &'static mut [usize] =
        unsafe { core::slice::from_raw_parts_mut(bitmap_va.as_mut_ptr(), bitmap_words) };

    // SAFETY: `managed` covers RAM strictly above the kernel image and the
    // bitmap — memory nothing else owns — and the boot table maps every frame in it.
    let mut allocator = unsafe {
        FrameAllocator::new(managed, bitmap).expect("frame allocator initialization failed")
    };
    reserve_foreign_memory(&mut allocator, managed);
    *FRAME_ALLOCATOR.lock() = Some(allocator);

    report_reservations();

    // The span taken, bitmap included, so `[start_ppn, end_ppn)` rather than `managed`.
    // This is what `kernel_table` maps; see `owned_range`.
    OWNED_START.store(PhysicalAddr::from_ppn(start_ppn).bits(), Ordering::Relaxed);
    OWNED_END.store(PhysicalAddr::from_ppn(end_ppn).bits(), Ordering::Relaxed);
}

/// Longest reservation label kept. Device-tree node names reach ~20 characters.
const RESERVATION_NAME_LEN: usize = 40;

/// Reservations recordable: the device-tree blob plus every firmware carve-out.
const MAX_RESERVATIONS: usize = 24;

/// One physical range withheld from the pool, and what withheld it.
#[derive(Clone, Debug)]
pub struct Reservation {
    name: String<RESERVATION_NAME_LEN>,
    /// First withheld physical address.
    pub start: PhysicalAddr,
    /// Exclusive end.
    pub end: PhysicalAddr,
    /// Frames this record was the first to withhold.
    ///
    /// Not derivable from `start..end`: carve-outs may overlap, so the extents summed
    /// exceed the memory actually removed from the pool. A record whose extent exceeds
    /// this is one that overlapped an earlier one.
    pub newly_withheld: usize,
}

impl Reservation {
    /// Why this range is withheld.
    pub fn name(&self) -> &str { &self.name }

    /// Frames this range spans, overlap included.
    pub fn frames(&self) -> usize { self.end.sub_addr(self.start) / PAGE_SIZE }
}

/// Everything withheld from the pool, in the order it was withheld.
///
/// A reserved frame and an allocated frame are indistinguishable in the bitmap — that
/// is what makes reclaiming an initrd a plain `deallocate_at` — so without this record
/// nothing could answer "why is this memory not free?", and a leak of 200 frames would
/// look exactly like a firmware carve-out of 200 frames.
static RESERVATIONS: Mutex<Vec<Reservation, MAX_RESERVATIONS>> = Mutex::new(Vec::new());

/// Every range withheld from the pool, cloned out so no lock is held by the caller.
pub fn reservations() -> Vec<Reservation, MAX_RESERVATIONS> { RESERVATIONS.lock().clone() }

/// Withhold `[start, end)` from the pool, recording it as `name`.
///
/// Rounded **outward** to whole frames: a partially covered frame is still a frame
/// that must not be handed out.
///
/// Only the overlap with the pool is withheld. A carve-out need not be inside it —
/// on QEMU virt the firmware's own reservations sit below the kernel image entirely —
/// and a range that misses the pool is reported rather than silently ignored, because
/// "outside" and "forgot to reserve" must not look the same.
fn reserve(
    allocator: &mut FrameAllocator<'static>,
    managed: FrameRange,
    withheld: &mut Vec<FrameRange, MAX_RESERVATIONS>,
    name: &str,
    start: PhysicalAddr,
    end: PhysicalAddr,
) {
    let first = start.align_down(PAGE_SIZE).ppn().max(managed.start());
    let last = end.align_up(PAGE_SIZE).ppn().min(managed.end());

    let Ok(range) = FrameRange::new(first, last) else {
        println!("[memory] reserve: {name} at {start:#x}..{end:#x} is outside the pool, skipped");
        return;
    };

    // Frame at a time, skipping what an earlier carve-out already withheld.
    //
    // `FrameAllocator::reserve` rejects an already-claimed frame, correctly: reserving
    // memory that has been *vended* is a genuine conflict. Overlap between carve-outs is
    // not, and this function manufactures it — the rounding above is OUTWARD, so two
    // carve-outs a few hundred bytes apart land in the same frame. Firmware also supplies
    // genuine duplicates, describing one reservation through both the FDT rsvmap and a
    // /reserved-memory node.
    //
    // Disjointness belongs here because the rounding that destroys it is here, and
    // because merging in the device tree would lose the names — which are how a later
    // reclaim finds the initrd.
    let free_before = allocator.free_frames();
    let mut newly = 0;
    for frame in range.start()..range.end() {
        if withheld.iter().any(|held| held.start() <= frame && frame < held.end()) {
            continue;
        }
        let single = FrameRange::new(frame, frame + 1).expect("frame + 1 always exceeds frame");
        allocator.reserve(single).unwrap_or_else(|error| {
            panic!("reserving {name} at {start:#x}..{end:#x} failed at frame {frame}: {error}")
        });
        newly += 1;
    }
    if withheld.push(range).is_err() {
        // Only the overlap bookkeeping is lost, not the reservation itself. A later
        // range overlapping this one would then hit the allocator's rejection and
        // panic, so this has to be loud.
        println!(
            "[memory] WARNING: more than {MAX_RESERVATIONS} reservations; overlap detection \
             is now incomplete"
        );
    }

    // A reservation that withheld nothing new AND overlapped nothing would leave the
    // memory vendable, and that corruption surfaces nowhere near its cause. Checking
    // against `newly` makes an overlap legal without making a no-op legal.
    assert_eq!(
        allocator.free_frames(),
        free_before - newly,
        "reserving {newly} new frames for {name} did not remove them from the pool"
    );

    let record = Reservation {
        name: crate::utils::truncated(name),
        start: PhysicalAddr::from_ppn(range.start()),
        end: PhysicalAddr::from_ppn(range.end()),
        newly_withheld: newly,
    };
    if RESERVATIONS.lock().push(record).is_err() {
        // The frames are withheld either way; only the record is lost. Say so, since
        // the list is what the boot log and any future reclaim rely on.
        println!("[memory] WARNING: more than {MAX_RESERVATIONS} reservations; {name} unrecorded");
    }
}

/// Withhold every physical range that exists but is not the kernel's to hand out.
///
/// Two sources, both from the device tree, both of which the allocator would
/// otherwise vend:
///
/// 1. **The blob itself.** On QEMU virt it sits at `0x87e00000`, near the top of RAM
///    and squarely inside the pool.
/// 2. **`/reserved-memory`.** Firmware carve-outs — OpenSBI's own `mmode_resv0`/`1`
///    here. They happen to sit *below* the kernel image on this platform, so they
///    currently miss the pool and are safe by accident; firmware reserving above the
///    kernel is entirely normal, and then they would not be.
fn reserve_foreign_memory(allocator: &mut FrameAllocator<'static>, managed: FrameRange) {
    let foreign = crate::device_tree::foreign_ram();
    assert!(
        !foreign.is_empty(),
        "no foreign RAM discovered, not even the device-tree blob; \
         call device_tree::init before memory::init"
    );
    // The frame ranges withheld so far, so a later carve-out overlapping an earlier one
    // is recognised rather than rejected. Local: it only makes the sequence of calls
    // order-independent.
    let mut withheld: Vec<FrameRange, MAX_RESERVATIONS> = Vec::new();
    for range in foreign {
        // The device tree reports raw integers; this is where they become addresses.
        reserve(
            allocator,
            managed,
            &mut withheld,
            range.name(),
            PhysicalAddr::new(range.base),
            PhysicalAddr::new(range.end()),
        );
    }
}

/// Print what was withheld, from the record rather than from each call site.
fn report_reservations() {
    let reserved = reservations();
    // Summed over what each record removed, not over its extent: carve-outs may
    // describe the same memory twice, and adding extents would overstate the loss.
    let frames: usize = reserved.iter().map(|entry| entry.newly_withheld).sum();
    println!("[memory] withheld {} frames in {} reservations:", frames, reserved.len());
    for entry in &reserved {
        // A record that withheld less than it spans overlapped an earlier one; mark
        // it so the total reconciles against these lines.
        let overlap = entry.frames() - entry.newly_withheld;
        let note = if overlap > 0 { " (already covered)" } else { "" };
        println!(
            "[memory]   {:<24} {:#x}..{:#x} ({}){}",
            entry.name(),
            entry.start,
            entry.end,
            ByteSize(entry.end.sub_addr(entry.start)),
            note
        );
    }
}

/// Allocate one zeroed physical frame, or `None` if the pool is exhausted.
pub fn alloc() -> Option<Frames> { alloc_contiguous(1) }

/// Allocate `count` physically contiguous zeroed frames, returning the run
/// (`count` is rounded up to a power of two internally, a buddy property, so the
/// base is aligned to the run's size).
pub fn alloc_contiguous(count: usize) -> Option<Frames> {
    let count = NonZeroUsize::new(count)?;
    let block = FRAME_ALLOCATOR.lock().as_mut()?.allocate(count)?;
    let base_va = phys_to_virt(PhysicalAddr::from_ppn(block.start_frame()));
    // SAFETY: the allocator just gave us exclusive ownership of these frames,
    // which are mapped writable through the high half.
    unsafe {
        core::ptr::write_bytes(base_va.as_mut_ptr::<u8>(), 0, block.frame_count() * PAGE_SIZE)
    };
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
/// [`FrameSource::alloc_zeroed`] returns a bare [`PhysicalAddr`] and lets the
/// [`Frames`] token go. That is the handoff, not a leak: the moment the frame is
/// installed as a branch PTE, the *page table* becomes its record of ownership.
/// Reclaiming it means walking to that entry and passing the address it holds to
/// [`free_at`] — which is what [`FrameSource::free`] does below, and why [`free_at`]
/// exists at all.
pub struct TableFrames;

// SAFETY: `alloc` returns a page-aligned frame, freshly zeroed (see the module
// docs — zeroing is this module's policy, and a zeroed frame is what makes a new
// table read as "all entries invalid"), owned exclusively by the caller until it
// comes back through `free_at`.
unsafe impl FrameSource for TableFrames {
    fn alloc_zeroed(&mut self) -> Option<PhysicalAddr> { alloc().map(|frames| frames.base()) }

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
    let a_va = phys_to_virt(a_base);
    let first_byte = unsafe { core::ptr::read_volatile(a_va.as_ptr::<u8>()) };
    assert_eq!(first_byte, 0u8, "frame {a_base:?} was not zeroed on alloc");

    // (2) A second frame is distinct.
    let b = alloc().expect("frame self-test: pool empty on second alloc");
    assert!(a_base != b.base(), "frame allocator handed out {a_base:?} twice");

    // (3) Dirty then free `a`; if that frame comes back, it must be re-zeroed —
    //     proving alloc zeroes recycled frames, not just pristine RAM.
    unsafe { core::ptr::write_bytes(a_va.as_mut_ptr::<u8>(), 0xAB, PAGE_SIZE) };
    // SAFETY: `a` has no users in this host-free test.
    unsafe { free(a) };
    let c = alloc().expect("frame self-test: pool empty on realloc");
    if c.base() == a_base {
        let byte = unsafe { core::ptr::read_volatile(phys_to_virt(c.base()).as_ptr::<u8>()) };
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
