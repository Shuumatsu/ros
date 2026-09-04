//! Frame-pool reservations, including allocator metadata.

use frame_allocator::{FrameAllocator, FrameRange};
use heapless::Vec;

use mmu::PAGE_SIZE;
use mmu::{MemoryAddr, PhysicalAddr};

use crate::memory::machine::MAX_FOREIGN;
use crate::memory::phys_range::PhysRange;
use crate::sync::IrqMutex;
use crate::utils::ByteSize;

pub const MAX_CARVEOUTS: usize = MAX_FOREIGN + 1;

const MAX_RESERVATIONS: usize = MAX_CARVEOUTS + 1;

const METADATA: &str = "frame bitmap";

/// A range withheld from the frame pool.
#[derive(Clone, Debug)]
pub struct Reservation {
    /// Page-rounded range clipped to the pool.
    pub range: PhysRange,
    /// Frames not already covered by earlier overlapping reservations.
    pub newly_withheld: usize,
}

impl Reservation {
    pub fn frames(&self) -> usize { self.range.size / PAGE_SIZE }
}

static RESERVATIONS: IrqMutex<Vec<Reservation, MAX_RESERVATIONS>> = IrqMutex::new(Vec::new());

pub fn list() -> Vec<Reservation, MAX_RESERVATIONS> {
    RESERVATIONS.with(|reservations| reservations.clone())
}

fn ppn_span(range: &PhysRange) -> (usize, usize) {
    let (start, end) = range.footprint();
    (start.ppn(), end.ppn())
}

/// Find the lowest run in `pool` that does not intersect a page-rounded carve-out.
///
/// The bitmap is initialized before reservations can protect memory, so its placement must
/// already exclude every carve-out.
///
/// # Panics
///
/// Panics if no suitable run exists.
pub fn place_metadata(pool: FrameRange, frames: usize, carveouts: &[PhysRange]) -> FrameRange {
    let mut start = pool.start();
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

/// Withhold the page-rounded intersection of `carveout` and `pool`.
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
        println!(
            "[memory] WARNING: reserve: only {:#x}..{:#x} of {name} at {start:#x}..{end:#x} \
             lies in the pool; the rest is not the allocator's to withhold",
            PhysicalAddr::from_ppn(first),
            PhysicalAddr::from_ppn(last)
        );
    }

    // Overlapping carve-outs share frames without double-reserving them.
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

/// Withhold allocator metadata and all carve-outs before publishing the allocator.
///
/// Metadata is recorded first; later overlaps are treated as already covered.
pub fn withhold_all(
    allocator: &mut FrameAllocator<'static>,
    pool: FrameRange,
    metadata: FrameRange,
    carveouts: &[PhysRange],
) {
    let mut withheld: Vec<FrameRange, MAX_RESERVATIONS> = Vec::new();

    let base = PhysicalAddr::from_ppn(metadata.start());
    let size = PhysicalAddr::from_ppn(metadata.end()).sub_addr(base);
    withhold(allocator, pool, &mut withheld, &PhysRange::new(METADATA, base, size));

    for range in carveouts {
        withhold(allocator, pool, &mut withheld, range);
    }
}

pub fn report() {
    let reserved = list();
    let frames: usize = reserved.iter().map(|entry| entry.newly_withheld).sum();
    println!("[memory]   withheld {} frames in {} reservations:", frames, reserved.len());
    for entry in &reserved {
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
