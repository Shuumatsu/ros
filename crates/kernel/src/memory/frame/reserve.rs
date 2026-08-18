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

use mmu::PAGE_SIZE;
use mmu::{MemoryAddr, PhysicalAddr};

use crate::memory::machine::MAX_FOREIGN;
use crate::memory::phys_range::PhysRange;
use crate::sync::IrqMutex;
use crate::utils::ByteSize;

/// Carve-outs the pool can hold: every foreign range the machine describes, plus the
/// kernel image, which no machine describes and [`super::init`] contributes.
pub const MAX_CARVEOUTS: usize = MAX_FOREIGN + 1;

/// Reservations recordable: every carve-out, plus the allocator's own metadata. Stated
/// against [`MAX_CARVEOUTS`] so the record cannot be shorter than what it records.
const MAX_RESERVATIONS: usize = MAX_CARVEOUTS + 1;

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

/// [`PhysRange::footprint`] as page numbers, which is what the allocator speaks.
///
/// Named for what it returns rather than repeating `footprint`: everything below compares
/// frame numbers, and a helper that shares its caller's name while changing its units is a
/// helper you have to read twice.
fn ppn_span(range: &PhysRange) -> (usize, usize) {
    let (start, end) = range.footprint();
    (start.ppn(), end.ppn())
}

/// Choose where the allocator's metadata bitmap goes: the lowest run of `frames` frames in
/// `pool` that no carve-out claims.
///
/// The bitmap is *written* — zeroed — the moment the allocator is built, which is before
/// any reservation can exist to protect anything from it. The pool starts at the base of
/// the RAM bank, so its first frames are the firmware's own, and its lowest free run is
/// wherever the previous boot stage stopped.
///
/// The lowest such run, so the metadata stays out of the way of large contiguous
/// allocations.
///
/// # Panics
///
/// If the pool has no room for the bitmap outside the carve-outs — the kernel cannot
/// manage memory it has nowhere to keep the bookkeeping for.
pub fn place_metadata(pool: FrameRange, frames: usize, carveouts: &[PhysRange]) -> FrameRange {
    let mut start = pool.start();
    // Each pass either settles or steps `start` past the end of one carve-out, and a range
    // once stepped over is never stepped over again, so the list length bounds the number
    // of restarts.
    for _ in 0..=carveouts.len() {
        let end = start.saturating_add(frames);
        assert!(
            end <= pool.end(),
            "no room in the {} frame pool for a {frames}-frame allocator bitmap outside the \
             {} ranges withheld from it",
            pool.len(),
            carveouts.len()
        );

        match carveouts.iter().find(|range| {
            let (first, last) = ppn_span(range);
            first < end && start < last
        }) {
            Some(conflict) => start = ppn_span(conflict).1,
            None => return FrameRange::new(start, end).expect("a bitmap spans at least one frame"),
        }
    }
    unreachable!("stepping past every carve-out leaves nothing left to conflict with")
}

/// Withhold `carveout` from the pool, recording it under its own name.
///
/// Rounded **outward**: a partially covered frame is still a frame that must not be
/// handed out. Only the overlap with the pool is withheld; a range that misses it
/// entirely and one that straddles its edge are both reported, because "outside",
/// "half inside" and "forgot to reserve" must not look the same in the log.
fn withhold(
    allocator: &mut FrameAllocator<'static>,
    pool: FrameRange,
    withheld: &mut Vec<FrameRange, MAX_RESERVATIONS>,
    carveout: &PhysRange,
) {
    let (start, end) = (carveout.base, carveout.end());
    let name = carveout.name();
    let (wanted_first, wanted_last) = ppn_span(carveout);
    let first = wanted_first.max(pool.start());
    let last = wanted_last.min(pool.end());

    let Ok(range) = FrameRange::new(first, last) else {
        println!("[memory] reserve: {name} at {start:#x}..{end:#x} is outside the pool, skipped");
        return;
    };
    if first > wanted_first || last < wanted_last {
        // Loud, because this is what a carve-out straddling the edge of the RAM bank looks
        // like — the top of it, once the direct map's window has clipped the pool — and the
        // record below would otherwise report the clipped range as if it were the whole.
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
    withheld.push(range).expect("one reservation per carve-out, plus the metadata");

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
    RESERVATIONS
        .with(|reservations| reservations.push(record))
        .expect("one reservation per carve-out, plus the metadata");
}

/// Withhold everything in `pool` that is not the kernel's to vend: the allocator's own
/// `metadata`, then the kernel image, the device-tree blob, firmware carve-outs and an
/// initrd.
///
/// Metadata first, since it is the one range already known to conflict with nothing —
/// [`place_metadata`] chose it that way — so a later carve-out overlapping it is
/// recognised as overlap rather than rejected as a double reservation.
///
/// Takes the allocator by reference rather than reaching for the global one: this runs
/// before publication, so no frame can be vended in between.
pub fn withhold_all(
    allocator: &mut FrameAllocator<'static>,
    pool: FrameRange,
    metadata: FrameRange,
    carveouts: &[PhysRange],
) {
    // The frame ranges withheld so far, so a later carve-out overlapping an earlier one
    // is recognised rather than rejected. Local: it only makes the sequence of calls
    // order-independent.
    let mut withheld: Vec<FrameRange, MAX_RESERVATIONS> = Vec::new();

    let base = PhysicalAddr::from_ppn(metadata.start());
    let size = PhysicalAddr::from_ppn(metadata.end()).sub_addr(base);
    withhold(allocator, pool, &mut withheld, &PhysRange::new(METADATA, base, size));

    for range in carveouts {
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
