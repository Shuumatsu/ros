//! The growth policy, exercised on host memory: when the heap asks for more, how much it
//! asks for, and when it refuses to ask.

use core::alloc::Layout;

use pretty_assertions::assert_eq;

use heap::{GrowableHeap, Limits, Outcome};

/// Blocks up to 2^15 = 32 KiB, which is plenty for these sizes and keeps the free-list
/// array small.
const ORDER: usize = 16;

const KIB: usize = 1024;

macro_rules! check {
    ($condition:expr, $($message:tt)+) => {
        if !$condition {
            panic!($($message)+);
        }
    };
}

/// Page-aligned host memory to hand the heap, in power-of-two runs aligned to their own
/// size — the same shape a buddy frame allocator vends, which is what the growth
/// arithmetic is written against.
struct Arena {
    regions: Vec<Vec<u8>>,
}

impl Arena {
    fn new() -> Self { Self { regions: Vec::new() } }

    /// Reserve `len` bytes (rounded up to a power of two) and return where they start.
    fn supply(&mut self, len: usize) -> (usize, usize) {
        let len = len.next_power_of_two();
        // Over-allocate so a base aligned to `len` is available inside the block, which a
        // plain `Vec<u8>` does not promise on its own.
        let mut region = vec![0u8; len * 2];
        let base = region.as_mut_ptr() as usize;
        let aligned = (base + len - 1) & !(len - 1);
        self.regions.push(region);
        (aligned, len)
    }
}

fn limits(initial: usize, step: usize, max: usize) -> Limits { Limits { initial, step, max } }

fn layout(size: usize) -> Layout {
    Layout::from_size_align(size, align_of::<usize>()).expect("test layout must be valid")
}

/// Give a configured heap its initial memory, as the kernel's `heap::init` does.
fn started(limits: Limits, arena: &mut Arena) -> GrowableHeap<ORDER> {
    let mut heap = GrowableHeap::<ORDER>::new();
    heap.configure(limits);
    let (start, len) = arena.supply(limits.initial);
    // SAFETY: the arena owns this range for the whole test and hands it out once.
    unsafe { heap.add_region(start, len) };
    heap
}

#[test]
fn serves_from_what_it_has_and_reports_occupancy() {
    let mut arena = Arena::new();
    let mut heap = started(limits(4 * KIB, 4 * KIB, 64 * KIB), &mut arena);
    assert_eq!(heap.stats().total, 4 * KIB, "the heap must report the memory it was given");
    assert_eq!(heap.stats().used, 0, "a fresh heap has nothing handed out");

    let Outcome::Served(block) = heap.allocate(layout(512)) else {
        panic!("a 512-byte request must be served from a 4 KiB heap");
    };
    check!(heap.stats().used >= 512, "a served request must show up as used");

    // SAFETY: the block came from this heap with this layout and is freed once.
    unsafe { heap.deallocate(block, layout(512)) };
    assert_eq!(heap.stats().used, 0, "freeing the only block must empty the heap again");
}

#[test]
fn asks_for_the_step_when_a_small_request_finds_it_dry() {
    let mut arena = Arena::new();
    let step = 8 * KIB;
    let mut heap = started(limits(4 * KIB, step, 64 * KIB), &mut arena);

    // Drain it: one 4 KiB block is the whole heap.
    let Outcome::Served(_) = heap.allocate(layout(4 * KIB)) else {
        panic!("the initial region must serve one block of its own size");
    };

    let Outcome::Grow { at_least } = heap.allocate(layout(64)) else {
        panic!("a dry heap below its ceiling must ask for memory");
    };
    assert_eq!(at_least, step, "a request smaller than the step must still ask for the step");
}

/// A request larger than the step must widen the heap by enough to serve it, or the retry
/// after growing comes up dry a second time and the allocation fails with memory to spare.
#[test]
fn asks_for_the_whole_request_when_it_exceeds_the_step() {
    let mut arena = Arena::new();
    let mut heap = started(limits(4 * KIB, KIB, 128 * KIB), &mut arena);

    let big = 12 * KIB;
    let Outcome::Grow { at_least } = heap.allocate(layout(big)) else {
        panic!("a request larger than the heap must ask for memory");
    };
    assert_eq!(
        at_least,
        16 * KIB,
        "the ask must cover the buddy block a {big}-byte request is served from, not just its size"
    );

    // And growing by exactly that much is enough: one retry, then served.
    let (start, len) = arena.supply(at_least);
    // SAFETY: fresh arena memory, handed to this heap once.
    unsafe { heap.add_region(start, len) };
    let Outcome::Served(_) = heap.allocate(layout(big)) else {
        panic!("one growth of the amount asked for must satisfy the request");
    };
}

#[test]
fn honours_alignment_when_sizing_the_ask() {
    let mut arena = Arena::new();
    let mut heap = started(limits(4 * KIB, KIB, 128 * KIB), &mut arena);
    let aligned = Layout::from_size_align(64, 8 * KIB).expect("test layout must be valid");

    let Outcome::Grow { at_least } = heap.allocate(aligned) else {
        panic!("a heap with no suitably aligned block must ask for memory");
    };
    assert_eq!(
        at_least,
        8 * KIB,
        "a 64-byte request at 8 KiB alignment needs an 8 KiB block, not a 64-byte one"
    );
}

#[test]
fn refuses_to_ask_past_its_ceiling() {
    let mut arena = Arena::new();
    let block = 4 * KIB;
    let max = 2 * block;
    let mut heap = started(limits(block, block, max), &mut arena);

    // The initial region is one block, so this empties the heap without growing it.
    let Outcome::Served(_) = heap.allocate(layout(block)) else {
        panic!("the initial region must serve one block of its own size");
    };

    // Growing once lands exactly on the ceiling, which must still be allowed.
    let Outcome::Grow { at_least } = heap.allocate(layout(block)) else {
        panic!("growth that reaches the ceiling exactly must be allowed");
    };
    assert_eq!(at_least, block, "the ask must be one block");
    let (start, len) = arena.supply(at_least);
    // SAFETY: fresh arena memory, handed to this heap once.
    unsafe { heap.add_region(start, len) };
    let Outcome::Served(_) = heap.allocate(layout(block)) else {
        panic!("the grown heap must serve the request that caused the growth");
    };
    assert_eq!(heap.stats().total, max, "the heap must now be exactly at its ceiling");

    // Anything that needs more memory is now refused, with what it would have taken.
    match heap.allocate(layout(block)) {
        Outcome::AtCeiling { wanted } => {
            assert_eq!(wanted, block, "the refusal must report what the request needed")
        }
        other => panic!("a heap at its ceiling must refuse to grow, got {other:?}"),
    }
}

#[test]
#[should_panic(expected = "before the heap is given memory")]
fn rejects_limits_changed_under_a_live_heap() {
    let mut arena = Arena::new();
    let mut heap = started(limits(4 * KIB, KIB, 64 * KIB), &mut arena);
    heap.configure(limits(4 * KIB, KIB, 128 * KIB));
}

#[test]
#[should_panic(expected = "grows by nothing")]
fn rejects_a_zero_step() { GrowableHeap::<ORDER>::new().configure(limits(4 * KIB, 0, 64 * KIB)); }

#[test]
#[should_panic(expected = "ceiling")]
fn rejects_a_ceiling_below_the_initial_size() {
    GrowableHeap::<ORDER>::new().configure(limits(8 * KIB, KIB, 4 * KIB));
}
