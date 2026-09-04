//! Frame-pool reservations, including allocator metadata.

use frame_allocator::{FrameAllocator, FrameRange};
use heapless::Vec;

use mmu::PAGE_SIZE;
use mmu::PhysicalAddr;

use crate::memory::machine::MAX_FOREIGN;
use crate::memory::phys_range::PhysRange;
use crate::sync::IrqMutex;
use crate::utils::ByteSize;

pub const MAX_CARVEOUTS: usize = MAX_FOREIGN + 1;

const MAX_RESERVATIONS: usize = MAX_CARVEOUTS + 1;

const METADATA: &str = "frame bitmap";

/// A range withheld from the frame pool.
#[derive(Clone, Debug)]
struct Reservation {
    /// Page-rounded range clipped to the pool.
    range: PhysRange,
    /// Frames not already covered by earlier overlapping reservations.
    newly_withheld: usize,
}

impl Reservation {
    fn frames(&self) -> usize { self.range.size / PAGE_SIZE }
}

static RESERVATIONS: IrqMutex<Vec<Reservation, MAX_RESERVATIONS>> = IrqMutex::new(Vec::new());

/// The byte range `frames` covers, under `name`.
pub fn phys_range(name: &str, frames: FrameRange) -> PhysRange {
    PhysRange::new(name, PhysicalAddr::from_ppn(frames.start()), frames.len() * PAGE_SIZE)
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
    // Every conflict moves `start` past that carve-out, so this runs at most once per carve-out.
    let mut start = pool.start();
    loop {
        let end = start.saturating_add(frames);
        assert!(
            end <= pool.end(),
            "no room in the {} frame pool for a {frames}-frame allocator bitmap outside the \
             {} ranges withheld from it",
            pool.len(),
            carveouts.len()
        );

        let conflict = carveouts.iter().find_map(|range| {
            let (first, last) = ppn_span(range);
            (first < end && start < last).then_some(last)
        });
        let Some(past_conflict) = conflict else {
            return FrameRange::new(start, end).expect("a bitmap spans at least one frame");
        };
        start = past_conflict;
    }
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

    // Overlapping carve-outs share frames, so reserve only the runs no earlier one covers.
    let free_before = allocator.free_frames();
    let mut newly = 0;
    let mut cursor = range.start();
    while cursor < range.end() {
        while let Some(covering) = withheld.iter().find(|held| held.contains(cursor)) {
            cursor = covering.end();
        }
        if cursor >= range.end() {
            break;
        }
        let run_end = withheld
            .iter()
            .map(|held| held.start())
            .filter(|&first| first > cursor)
            .min()
            .unwrap_or(range.end())
            .min(range.end());
        let run = FrameRange::new(cursor, run_end).expect("a run spans at least one frame");
        allocator.reserve(run).unwrap_or_else(|error| {
            panic!("reserving {name} at {start:#x}..{end:#x} failed at frame {cursor}: {error}")
        });
        newly += run_end - cursor;
        cursor = run_end;
    }
    withheld.push(range).expect("one reservation per carve-out, plus the metadata");

    assert_eq!(
        allocator.free_frames(),
        free_before - newly,
        "reserving {newly} new frames for {name} did not remove them from the pool"
    );

    let record = Reservation { range: phys_range(name, range), newly_withheld: newly };
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

    withhold(allocator, pool, &mut withheld, &phys_range(METADATA, metadata));

    for range in carveouts {
        withhold(allocator, pool, &mut withheld, range);
    }
}

pub fn report() {
    RESERVATIONS.with(|reserved| {
        let frames: usize = reserved.iter().map(|entry| entry.newly_withheld).sum();
        println!("[memory]   withheld {} frames in {} reservations:", frames, reserved.len());
        for entry in reserved.iter() {
            let note =
                if entry.newly_withheld < entry.frames() { " (already covered)" } else { "" };
            println!(
                "[memory]     {:<24} {:#x}..{:#x} ({}){}",
                entry.range.name(),
                entry.range.base,
                entry.range.end(),
                ByteSize(entry.range.size),
                note
            );
        }
    });
}
