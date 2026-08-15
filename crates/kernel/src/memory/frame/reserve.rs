//! Physical memory that exists but is not the kernel's to hand out.
//!
//! A reserved frame and an allocated one are indistinguishable in the allocator's bitmap
//! — which is what makes reclaiming an initrd later a plain `deallocate_at`. Hence the
//! record below: without it, a leak of 200 frames looks exactly like a firmware
//! carve-out of 200 frames.
//!
//! The allocator's own metadata is withheld the same way, through [`place_metadata`] and
//! then the ordinary path. It has to be: the bitmap is bytes written into the pool before
//! a single reservation can be expressed, so where it goes is a decision that must already
//! respect them.

use frame_allocator::{FrameAllocator, FrameRange};
use heapless::Vec;

use paging::MemoryAddr;
use paging::sv39::{PAGE_SIZE, PhysicalAddr};

use crate::memory::machine::{MAX_FOREIGN, PhysRange};
use crate::sync::IrqMutex;
use crate::utils::ByteSize;

/// Reservations recordable: every foreign range, plus the allocator's own metadata.
const MAX_RESERVATIONS: usize = MAX_FOREIGN + 1;

/// What the allocator's metadata is called wherever it is reported.
const METADATA: &str = "frame bitmap";

/// One physical range withheld from the pool, and what withheld it.
#[derive(Clone, Debug)]
pub struct Reservation {
    /// The range as actually withheld: rounded outward to whole frames and clipped to the
    /// pool, so it is what left the allocator rather than what was asked for.
    pub range: PhysRange,
    /// Frames this record was the first to withhold. Not derivable from `range`:
    /// carve-outs may overlap, so summed extents exceed what left the pool.
    pub newly_withheld: usize,
}

impl Reservation {
    /// Frames this range spans, overlap included.
    pub fn frames(&self) -> usize { self.range.size / PAGE_SIZE }
}

/// Everything withheld from the pool, in the order it was withheld.
static RESERVATIONS: IrqMutex<Vec<Reservation, MAX_RESERVATIONS>> = IrqMutex::new(Vec::new());

/// Every range withheld from the pool, cloned out so no lock is held by the caller.
pub fn list() -> Vec<Reservation, MAX_RESERVATIONS> {
    RESERVATIONS.with(|reservations| reservations.clone())
}

/// The frames a range covers once rounded outward: a partially covered frame is still a
/// frame that must not be handed out.
fn frame_span(range: &PhysRange) -> (usize, usize) {
    let (start, end) = range.frame_span();
    (start.ppn(), end.ppn())
}

/// Choose where the allocator's metadata bitmap goes: the lowest run of `frames` frames in
/// `pool` that no foreign range claims.
///
/// The bitmap is *written* — zeroed — the moment the allocator is built, which is before
/// any reservation can exist to protect anything from it. Placing it at the front of the
/// pool unconditionally therefore destroys whatever the firmware left there, and an initrd
/// or a carve-out immediately above the kernel image is an entirely ordinary thing for a
/// previous boot stage to have produced.
///
/// The lowest such run, so the metadata stays out of the way of large contiguous
/// allocations, and so a machine with no carve-outs above the kernel gets the same layout
/// it always had.
///
/// # Panics
///
/// If the pool has no room for the bitmap outside the foreign ranges — the kernel cannot
/// manage memory it has nowhere to keep the bookkeeping for.
pub fn place_metadata(pool: FrameRange, frames: usize, foreign: &[PhysRange]) -> FrameRange {
    let mut start = pool.start();
    // Each pass either settles or steps `start` past the end of one foreign range, and a
    // range once stepped over is never stepped over again, so the list length bounds the
    // number of restarts.
    for _ in 0..=foreign.len() {
        let end = start.saturating_add(frames);
        assert!(
            end <= pool.end(),
            "no room in the {} frame pool for a {frames}-frame allocator bitmap outside the \
             {} ranges the machine reserved",
            pool.len(),
            foreign.len()
        );

        match foreign.iter().find(|range| {
            let (first, last) = frame_span(range);
            first < end && start < last
        }) {
            Some(conflict) => start = frame_span(conflict).1,
            None => return FrameRange::new(start, end).expect("a bitmap spans at least one frame"),
        }
    }
    unreachable!("stepping past every foreign range leaves nothing left to conflict with")
}

/// Withhold `foreign` from the pool, recording it under its own name.
///
/// Rounded **outward**: a partially covered frame is still a frame that must not be
/// handed out. Only the overlap with the pool is withheld; a range that misses it
/// entirely and one that straddles its edge are both reported, because "outside",
/// "half inside" and "forgot to reserve" must not look the same in the log.
fn withhold(
    allocator: &mut FrameAllocator<'static>,
    pool: FrameRange,
    withheld: &mut Vec<FrameRange, MAX_RESERVATIONS>,
    foreign: &PhysRange,
) {
    let (start, end) = (foreign.base, foreign.end());
    let name = foreign.name();
    let (wanted_first, wanted_last) = frame_span(foreign);
    let first = wanted_first.max(pool.start());
    let last = wanted_last.min(pool.end());

    let Ok(range) = FrameRange::new(first, last) else {
        println!("[memory] reserve: {name} at {start:#x}..{end:#x} is outside the pool, skipped");
        return;
    };
    if first > wanted_first || last < wanted_last {
        // Loud, because this is what a carve-out straddling the top of the kernel image
        // looks like, and the record below would otherwise report the clipped range as if
        // it were the whole of it.
        println!(
            "[memory] WARNING: reserve: only {:#x}..{:#x} of {name} at {start:#x}..{end:#x} \
             lies in the pool; the rest is not the allocator's to withhold",
            PhysicalAddr::from_ppn(first),
            PhysicalAddr::from_ppn(last)
        );
    }

    // Frame at a time, skipping what an earlier carve-out already withheld. The
    // allocator rejects an already-claimed frame, correctly — reserving *vended* memory
    // is a real conflict — but overlap between carve-outs is not, and the outward
    // rounding above manufactures it. Firmware supplies genuine duplicates too, via both
    // the FDT rsvmap and a /reserved-memory node. Disjointness belongs here rather than
    // in whoever described the machine, which would have to lose the names to merge.
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

    // Against `newly`, not the extent: that makes an overlap legal without making a
    // silent no-op legal, and a no-op would leave the memory vendable.
    assert_eq!(
        allocator.free_frames(),
        free_before - newly,
        "reserving {newly} new frames for {name} did not remove them from the pool"
    );

    let withheld_base = PhysicalAddr::from_ppn(range.start());
    let record = Reservation {
        range: PhysRange::new(
            name,
            withheld_base,
            PhysicalAddr::from_ppn(range.end()).sub_addr(withheld_base),
        ),
        newly_withheld: newly,
    };
    if RESERVATIONS.with(|reservations| reservations.push(record)).is_err() {
        // The frames are withheld either way; only the record is lost. Say so, since
        // the list is what the boot log and any future reclaim rely on.
        println!("[memory] WARNING: more than {MAX_RESERVATIONS} reservations; {name} unrecorded");
    }
}

/// Withhold everything in `pool` that is not the kernel's to vend: the allocator's own
/// `metadata`, then the device-tree blob, firmware carve-outs and an initrd.
///
/// Metadata first, since it is the one range already known to conflict with nothing —
/// [`place_metadata`] chose it that way — so a later carve-out overlapping it is
/// recognised as overlap rather than rejected as a double reservation.
///
/// Takes the allocator by reference rather than reaching for the global one: this runs
/// before publication, so no frame can be vended in between.
pub fn everything_foreign(
    allocator: &mut FrameAllocator<'static>,
    pool: FrameRange,
    metadata: FrameRange,
    foreign: &[PhysRange],
) {
    assert!(
        !foreign.is_empty(),
        "the machine reports no foreign RAM at all, not even a device-tree blob; \
         something described it wrong"
    );
    // The frame ranges withheld so far, so a later carve-out overlapping an earlier one
    // is recognised rather than rejected. Local: it only makes the sequence of calls
    // order-independent.
    let mut withheld: Vec<FrameRange, MAX_RESERVATIONS> = Vec::new();

    let base = PhysicalAddr::from_ppn(metadata.start());
    let size = PhysicalAddr::from_ppn(metadata.end()).sub_addr(base);
    withhold(allocator, pool, &mut withheld, &PhysRange::new(METADATA, base, size));

    for range in foreign {
        withhold(allocator, pool, &mut withheld, range);
    }
}

/// Print what was withheld, from the record rather than from each call site.
pub fn report() {
    let reserved = list();
    // Summed over what each record removed, not over its extent: carve-outs may
    // describe the same memory twice, and adding extents would overstate the loss.
    let frames: usize = reserved.iter().map(|entry| entry.newly_withheld).sum();
    println!("[memory]   withheld {} frames in {} reservations:", frames, reserved.len());
    for entry in &reserved {
        // A record that withheld less than it spans overlapped an earlier one; mark
        // it so the total reconciles against these lines.
        let overlap = entry.frames() - entry.newly_withheld;
        let note = if overlap > 0 { " (already covered)" } else { "" };
        println!(
            "[memory]     {:<24} {:#x}..{:#x} ({}){}",
            entry.range.name(),
            entry.range.base,
            entry.range.end(),
            ByteSize(entry.range.size),
            note
        );
    }
}
