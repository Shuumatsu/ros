//! Physical memory that exists but is not the kernel's to hand out.
//!
//! A reserved frame and an allocated one are indistinguishable in the allocator's bitmap
//! — which is what makes reclaiming an initrd later a plain `deallocate_at`. Hence the
//! record below: without it, a leak of 200 frames looks exactly like a firmware
//! carve-out of 200 frames.

use frame_allocator::{FrameAllocator, FrameRange};
use heapless::{String, Vec};

use paging::MemoryAddr;
use paging::sv39::{PAGE_SIZE, PhysicalAddr};

use crate::sync::IrqMutex;
use crate::utils::ByteSize;

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
    /// Frames this record was the first to withhold. Not derivable from `start..end`:
    /// carve-outs may overlap, so summed extents exceed what left the pool.
    pub newly_withheld: usize,
}

impl Reservation {
    /// Why this range is withheld.
    pub fn name(&self) -> &str { &self.name }

    /// Frames this range spans, overlap included.
    pub fn frames(&self) -> usize { self.end.sub_addr(self.start) / PAGE_SIZE }
}

/// Everything withheld from the pool, in the order it was withheld.
static RESERVATIONS: IrqMutex<Vec<Reservation, MAX_RESERVATIONS>> = IrqMutex::new(Vec::new());

/// Every range withheld from the pool, cloned out so no lock is held by the caller.
pub fn list() -> Vec<Reservation, MAX_RESERVATIONS> {
    RESERVATIONS.with(|reservations| reservations.clone())
}

/// Withhold `[start, end)` from the pool, recording it as `name`.
///
/// Rounded **outward**: a partially covered frame is still a frame that must not be
/// handed out. Only the overlap with the pool is withheld, and a range that misses it
/// entirely is reported — "outside" and "forgot to reserve" must not look the same.
fn withhold(
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

    // Frame at a time, skipping what an earlier carve-out already withheld. The
    // allocator rejects an already-claimed frame, correctly — reserving *vended* memory
    // is a real conflict — but overlap between carve-outs is not, and the outward
    // rounding above manufactures it. Firmware supplies genuine duplicates too, via both
    // the FDT rsvmap and a /reserved-memory node. Disjointness belongs here rather than
    // in the device tree, which would have to lose the names to merge.
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

    let record = Reservation {
        name: crate::utils::truncated(name),
        start: PhysicalAddr::from_ppn(range.start()),
        end: PhysicalAddr::from_ppn(range.end()),
        newly_withheld: newly,
    };
    if RESERVATIONS.with(|reservations| reservations.push(record)).is_err() {
        // The frames are withheld either way; only the record is lost. Say so, since
        // the list is what the boot log and any future reclaim rely on.
        println!("[memory] WARNING: more than {MAX_RESERVATIONS} reservations; {name} unrecorded");
    }
}

/// Withhold every physical range the device tree reports as not ours: the blob itself,
/// firmware carve-outs, an initrd. On QEMU virt the blob is inside the pool and the
/// carve-outs are below it — safe by accident there, but firmware reserving above the
/// kernel is entirely normal.
///
/// Takes the allocator by reference rather than reaching for the global one: this runs
/// before publication, so no frame can be vended in between.
pub fn foreign_memory(allocator: &mut FrameAllocator<'static>, managed: FrameRange) {
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
        withhold(
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
            entry.name(),
            entry.start,
            entry.end,
            ByteSize(entry.end.sub_addr(entry.start)),
            note
        );
    }
}
