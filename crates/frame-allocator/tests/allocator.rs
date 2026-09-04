use core::num::NonZeroUsize;

use pretty_assertions::assert_eq;

use frame_allocator::{
    DeallocationError, FrameAllocator, FrameBlock, FrameRange, InitError, MetadataError,
    ReserveError, metadata_layout,
};

const TEST_METADATA_WORDS: usize = 64;

macro_rules! check {
    ($condition:expr, $($message:tt)+) => {
        if !$condition {
            panic!($($message)+);
        }
    };
}

fn range(start: usize, end: usize) -> FrameRange {
    FrameRange::new(start, end).unwrap_or_else(|error| panic!("invalid test range: {error}"))
}

fn count(value: usize) -> NonZeroUsize {
    NonZeroUsize::new(value).unwrap_or_else(|| panic!("test allocation count must be non-zero"))
}

fn allocator<'a>(frame_range: FrameRange, metadata: &'a mut [usize]) -> FrameAllocator<'a> {
    // SAFETY: each simulation owns its range and separate metadata.
    unsafe { FrameAllocator::new(frame_range, metadata).expect("allocator initialization failed") }
}

#[test]
fn rejects_empty_and_reversed_ranges() {
    let empty = FrameRange::new(7, 7).expect_err("empty range must fail");
    assert_eq!(empty.start(), 7, "empty range error lost its start");
    assert_eq!(empty.end(), 7, "empty range error lost its end");

    let reversed = FrameRange::new(9, 3).expect_err("reversed range must fail");
    assert_eq!(reversed.start(), 9, "reversed range error lost its start");
    assert_eq!(reversed.end(), 3, "reversed range error lost its end");
}

#[test]
fn error_messages_and_source_chain_are_stable() {
    use std::error::Error as _;

    let range_error = FrameRange::new(9, 3).expect_err("reversed range must fail");
    assert_eq!(
        range_error.to_string(),
        "frame range must be non-empty and increasing, got 9..3",
        "range error message changed"
    );

    let metadata_error =
        metadata_layout(range(0, usize::MAX)).expect_err("range must overflow metadata indexing");
    assert_eq!(
        metadata_error.to_string(),
        "frame metadata size exceeds usize",
        "metadata error message changed"
    );

    let wrapped = InitError::from(metadata_error);
    assert_eq!(
        wrapped.to_string(),
        "frame metadata size exceeds usize",
        "InitError::Metadata must display like its inner error"
    );
    let source = wrapped.source().expect("InitError::Metadata must expose a source");
    assert_eq!(
        source.to_string(),
        "frame metadata size exceeds usize",
        "InitError::Metadata source chain changed"
    );

    let insufficient = InitError::InsufficientMetadata { required: 4, provided: 1 };
    assert_eq!(
        insufficient.to_string(),
        "insufficient frame metadata: required 4 words, provided 1",
        "InsufficientMetadata message changed"
    );
    check!(insufficient.source().is_none(), "InsufficientMetadata must not carry a source");

    assert_eq!(
        DeallocationError::ForeignBlock.to_string(),
        "frame block does not belong to this allocator",
        "ForeignBlock message changed"
    );
    assert_eq!(
        DeallocationError::AlreadyFree.to_string(),
        "frame block is already free",
        "AlreadyFree message changed"
    );
    assert_eq!(
        DeallocationError::UnalignedFrame { start: 7, order: 2 }.to_string(),
        "frame 7 does not start an aligned block of order 2",
        "UnalignedFrame message changed"
    );
    assert_eq!(
        ReserveError::OutOfRange { start: 4, end: 9 }.to_string(),
        "reserved range 4..9 is not inside the managed range",
        "ReserveError::OutOfRange message changed"
    );
    assert_eq!(
        ReserveError::AlreadyAllocated { frame: 12 }.to_string(),
        "frame 12 is already allocated and cannot be reserved",
        "ReserveError::AlreadyAllocated message changed"
    );
}

#[test]
fn reports_exact_metadata_for_unaligned_range() {
    let layout = metadata_layout(range(3, 13)).expect("metadata layout must fit");
    assert_eq!(layout.roots(), 4, "unexpected buddy-root decomposition");
    assert_eq!(layout.bits(), 16, "metadata must use one bit per buddy-tree node");
    assert_eq!(
        layout.words(),
        16usize.div_ceil(usize::BITS as usize),
        "metadata word count must exactly cover its bits"
    );
}

#[test]
fn rejects_an_undersized_metadata_buffer() {
    let frame_range = range(0, 64);
    let required = metadata_layout(frame_range).expect("metadata layout must fit").words();
    let mut metadata = [0usize; 1];
    // SAFETY: the simulation owns the range; construction fails before allocation.
    let error = unsafe {
        FrameAllocator::new(frame_range, &mut metadata).err().expect("buffer must be rejected")
    };

    assert_eq!(
        error,
        InitError::InsufficientMetadata { required, provided: 1 },
        "initialization returned the wrong size error"
    );
}

#[test]
fn reports_unrepresentable_metadata_without_wrapping() {
    let error = metadata_layout(range(0, usize::MAX))
        .expect_err("near-address-space-sized range must overflow metadata indexing");
    assert_eq!(error, MetadataError::CapacityOverflow, "wrong metadata overflow diagnostic");
}

#[test]
fn allocates_every_frame_in_an_unaligned_range_once() {
    let frame_range = range(3, 13);
    let mut metadata = [usize::MAX; TEST_METADATA_WORDS];
    let mut allocator = allocator(frame_range, &mut metadata);
    let mut blocks: [Option<FrameBlock>; 10] = core::array::from_fn(|_| None);
    let mut starts = [usize::MAX; 10];

    for index in 0..blocks.len() {
        let block = allocator.allocate(count(1)).expect("single-frame allocation failed");
        let start = block.start_frame();
        check!(frame_range.contains(start), "allocator returned frame {start} out of range");
        check!(!starts[..index].contains(&start), "allocator returned duplicate frame {start}");
        starts[index] = start;
        blocks[index] = Some(block);
    }

    assert_eq!(allocator.free_frames(), 0, "all managed frames should be reserved");
    check!(allocator.allocate(count(1)).is_none(), "exhausted allocator returned another frame");

    for block in blocks {
        // SAFETY: each unique block has no live users and is returned once.
        unsafe {
            allocator
                .deallocate(block.expect("test lost an allocated block"))
                .expect("valid deallocation failed");
        }
    }
    assert_eq!(allocator.free_frames(), 10, "deallocation did not restore all frames");
}

#[test]
fn scans_and_coalesces_across_bitmap_word_boundaries() {
    let frame_range = range(0, 65);
    let mut metadata = [0usize; TEST_METADATA_WORDS];
    let mut allocator = allocator(frame_range, &mut metadata);
    let mut blocks: [Option<FrameBlock>; 65] = core::array::from_fn(|_| None);
    let mut seen = [false; 65];

    for slot in &mut blocks {
        let block = allocator.allocate(count(1)).expect("single-frame allocation failed");
        let index = block.start_frame();
        check!(!seen[index], "frame {index} was allocated twice");
        seen[index] = true;
        *slot = Some(block);
    }
    assert_eq!(seen, [true; 65], "word-boundary scan skipped managed frames");

    for index in (0..blocks.len()).step_by(2) {
        // SAFETY: each selected block is unique and has no users.
        unsafe {
            allocator
                .deallocate(blocks[index].take().expect("missing even-index block"))
                .expect("even-index deallocation failed");
        }
    }
    for index in (1..blocks.len()).step_by(2) {
        // SAFETY: each selected block is unique and has no users.
        unsafe {
            allocator
                .deallocate(blocks[index].take().expect("missing odd-index block"))
                .expect("odd-index deallocation failed");
        }
    }

    assert_eq!(allocator.free_frames(), 65, "cross-word frees lost capacity");
    let large = allocator.allocate(count(64)).expect("64-frame root did not coalesce");
    assert_eq!(large.start_frame(), 0, "coalesced root has the wrong base");
}

#[test]
fn rounds_contiguous_requests_and_preserves_global_alignment() {
    let mut metadata = [0usize; TEST_METADATA_WORDS];
    let mut allocator = allocator(range(3, 35), &mut metadata);
    let block = allocator.allocate(count(3)).expect("three-frame request should fit");

    assert_eq!(block.requested_frames(), 3, "requested size was not retained");
    assert_eq!(block.frame_count(), 4, "buddy allocation must round three frames to four");
    assert_eq!(
        block.start_frame() % block.frame_count(),
        0,
        "contiguous block is not globally aligned"
    );
}

#[test]
fn rejects_a_request_whose_rounding_overflows() {
    let mut metadata = [0usize; TEST_METADATA_WORDS];
    let mut allocator = allocator(range(0, 8), &mut metadata);

    check!(
        allocator.allocate(count(usize::MAX)).is_none(),
        "overflowing request unexpectedly allocated frames"
    );
    assert_eq!(allocator.free_frames(), 8, "overflowing request changed allocator state");
}

#[test]
fn coalesces_buddies_back_into_their_root() {
    let mut metadata = [0usize; TEST_METADATA_WORDS];
    let mut allocator = allocator(range(8, 16), &mut metadata);
    let left = allocator.allocate(count(4)).expect("left half allocation failed");
    let right = allocator.allocate(count(4)).expect("right half allocation failed");

    assert_eq!(left.start_frame(), 8, "unexpected left buddy");
    assert_eq!(right.start_frame(), 12, "unexpected right buddy");
    check!(allocator.allocate(count(1)).is_none(), "fully allocated root reported free space");

    // SAFETY: both allocator-owned blocks are distinct and have no users.
    unsafe {
        allocator.deallocate(left).expect("left buddy deallocation failed");
        allocator.deallocate(right).expect("right buddy deallocation failed");
    }

    let whole = allocator.allocate(count(8)).expect("coalesced root allocation failed");
    assert_eq!(whole.start_frame(), 8, "coalescing changed the root base");
    assert_eq!(whole.frame_count(), 8, "coalescing did not recover the full root");
}

#[test]
fn detects_a_duplicate_deallocation() {
    let mut metadata = [0usize; TEST_METADATA_WORDS];
    let mut allocator = allocator(range(0, 8), &mut metadata);
    let block = allocator.allocate(count(1)).expect("allocation failed");
    // SAFETY: `FrameBlock` has no destructor; the duplicate is used only for this check.
    let duplicate = unsafe { core::ptr::read(&block) };

    // SAFETY: the block is live, unused, and returned once.
    unsafe {
        allocator.deallocate(block).expect("initial deallocation failed");
    }
    // SAFETY: this intentionally violates the no-double-free contract to verify
    // rejection before allocator state changes.
    let error = unsafe {
        allocator.deallocate(duplicate).expect_err("duplicate deallocation was accepted")
    };
    assert_eq!(error, DeallocationError::AlreadyFree, "wrong duplicate-free diagnostic");
}

#[test]
fn rejects_a_block_from_another_range_layout() {
    let mut first_metadata = [0usize; TEST_METADATA_WORDS];
    let mut second_metadata = [0usize; TEST_METADATA_WORDS];
    let mut first = allocator(range(0, 8), &mut first_metadata);
    let mut second = allocator(range(8, 16), &mut second_metadata);
    let block = first.allocate(count(1)).expect("first allocator allocation failed");

    // SAFETY: this intentionally violates allocator provenance to verify structural
    // rejection before metadata changes.
    let error =
        unsafe { second.deallocate(block).expect_err("foreign block was unexpectedly accepted") };
    assert_eq!(error, DeallocationError::ForeignBlock, "wrong foreign-block diagnostic");
    assert_eq!(second.free_frames(), 8, "foreign deallocation changed allocator state");
}

#[test]
fn restores_fragmented_allocations_for_a_larger_request() {
    let mut metadata = [0usize; TEST_METADATA_WORDS];
    let mut allocator = allocator(range(0, 64), &mut metadata);
    let blocks = [
        allocator.allocate(count(1)).expect("one-frame allocation failed"),
        allocator.allocate(count(2)).expect("two-frame allocation failed"),
        allocator.allocate(count(3)).expect("three-frame allocation failed"),
        allocator.allocate(count(5)).expect("five-frame allocation failed"),
    ];

    assert_eq!(allocator.allocated_frames(), 15, "rounded allocation accounting is wrong");
    for block in blocks.into_iter().rev() {
        // SAFETY: each unique block has no users and is returned once.
        unsafe {
            allocator.deallocate(block).expect("fragment deallocation failed");
        }
    }

    assert_eq!(allocator.free_frames(), 64, "fragmented frees did not restore capacity");
    let whole = allocator.allocate(count(64)).expect("full-range allocation failed after merge");
    assert_eq!(whole.start_frame(), 0, "full-range allocation has the wrong base");
}

#[test]
fn reserved_frames_are_never_vended_even_under_exhaustion() {
    let mut metadata = [0usize; TEST_METADATA_WORDS];
    let mut allocator = allocator(range(0, 64), &mut metadata);
    allocator.reserve(range(21, 26)).expect("interior reservation must succeed");

    assert_eq!(allocator.free_frames(), 59, "reserving five frames must cost five frames");

    let mut seen = [false; 64];
    while let Some(block) = allocator.allocate(count(1)) {
        let frame = block.start_frame();
        check!(!(21..26).contains(&frame), "allocator handed out reserved frame {frame}");
        check!(!seen[frame], "frame {frame} was handed out twice");
        seen[frame] = true;
    }

    assert_eq!(allocator.free_frames(), 0, "pool should be drained");
    for frame in 0..64 {
        assert_eq!(
            seen[frame],
            !(21..26).contains(&frame),
            "frame {frame} was vended exactly when it should not have been (or vice versa)"
        );
    }
}

#[test]
fn reserving_splits_only_what_it_must() {
    let mut metadata = [0usize; TEST_METADATA_WORDS];
    let mut allocator = allocator(range(0, 64), &mut metadata);
    allocator.reserve(range(32, 33)).expect("single-frame reservation must succeed");

    assert_eq!(allocator.free_frames(), 63, "one reserved frame must cost exactly one");
    let half = allocator.allocate(count(32)).expect("the intact half must still be allocatable");
    assert_eq!(half.start_frame(), 0, "the intact half is the low one");
    assert_eq!(half.frame_count(), 32, "reserving must not have fragmented the other half");
}

#[test]
fn a_reserved_frame_can_be_reclaimed() {
    let mut metadata = [0usize; TEST_METADATA_WORDS];
    let mut allocator = allocator(range(0, 8), &mut metadata);
    allocator.reserve(range(3, 4)).expect("reservation must succeed");
    assert_eq!(allocator.free_frames(), 7, "reservation must be accounted");

    // SAFETY: frame 3 was reserved at order zero and has no users.
    unsafe {
        allocator.deallocate_at(3, 0).expect("a reserved frame must be reclaimable");
    }
    assert_eq!(allocator.free_frames(), 8, "reclaiming must restore the frame");
    let whole = allocator.allocate(count(8)).expect("root must coalesce after reclaim");
    assert_eq!(whole.frame_count(), 8, "reclaimed frame must merge back into the root");
}

#[test]
fn reserve_rejects_ranges_it_cannot_honour() {
    let mut metadata = [0usize; TEST_METADATA_WORDS];
    let mut allocator = allocator(range(8, 16), &mut metadata);

    for bad in [range(0, 4), range(4, 10), range(12, 20), range(20, 24)] {
        let error =
            allocator.reserve(bad).expect_err("a range outside the managed range must be rejected");
        assert_eq!(
            error,
            ReserveError::OutOfRange { start: bad.start(), end: bad.end() },
            "wrong diagnostic for out-of-range reservation {}..{}",
            bad.start(),
            bad.end()
        );
    }
    assert_eq!(allocator.free_frames(), 8, "rejected reservations must not change state");

    let block = allocator.allocate(count(1)).expect("allocation failed");
    let taken = block.start_frame();
    let error = allocator
        .reserve(range(taken, taken + 1))
        .expect_err("reserving an allocated frame must be rejected");
    assert_eq!(
        error,
        ReserveError::AlreadyAllocated { frame: taken },
        "wrong diagnostic for reserving allocated memory"
    );
}

#[test]
fn reserve_spans_multiple_buddy_roots() {
    let mut metadata = [usize::MAX; TEST_METADATA_WORDS];
    let mut allocator = allocator(range(3, 13), &mut metadata);
    allocator.reserve(range(6, 11)).expect("cross-root reservation must succeed");

    assert_eq!(allocator.free_frames(), 5, "five of ten frames remain");
    let mut seen = [false; 13];
    while let Some(block) = allocator.allocate(count(1)) {
        let frame = block.start_frame();
        check!(!(6..11).contains(&frame), "handed out reserved frame {frame}");
        seen[frame] = true;
    }
    for frame in 3..13 {
        assert_eq!(
            seen[frame],
            !(6..11).contains(&frame),
            "frame {frame} vended exactly when it should not have been (or vice versa)"
        );
    }
}

#[test]
fn deallocate_at_round_trips_a_single_frame() {
    let mut metadata = [0usize; TEST_METADATA_WORDS];
    let mut allocator = allocator(range(0, 8), &mut metadata);
    let block = allocator.allocate(count(1)).expect("single-frame allocation failed");
    let (start, order) = (block.start_frame(), block.order());
    assert_eq!(order, 0, "a one-frame request must be order 0");

    drop(block);
    assert_eq!(allocator.free_frames(), 7, "allocation accounting is wrong before the free");

    // SAFETY: the frame has its allocation order, no users, and is freed once.
    unsafe {
        allocator.deallocate_at(start, order).expect("address-based deallocation failed");
    }
    assert_eq!(allocator.free_frames(), 8, "freeing by address did not restore the frame");

    let whole = allocator.allocate(count(8)).expect("root did not coalesce after an address free");
    assert_eq!(whole.start_frame(), 0, "coalesced root has the wrong base");
}

#[test]
fn deallocate_at_is_indistinguishable_from_token_deallocation() {
    let mut token_metadata = [0usize; TEST_METADATA_WORDS];
    let mut address_metadata = [0usize; TEST_METADATA_WORDS];
    let mut by_token = allocator(range(0, 64), &mut token_metadata);
    let mut by_address = allocator(range(0, 64), &mut address_metadata);

    let requests = [1usize, 2, 3, 5];
    let mut tokens: [Option<FrameBlock>; 4] = core::array::from_fn(|_| None);
    let mut addresses = [(0usize, 0usize); 4];

    for (index, &request) in requests.iter().enumerate() {
        let token = by_token.allocate(count(request)).expect("token-side allocation failed");
        let mirror = by_address.allocate(count(request)).expect("address-side allocation failed");
        assert_eq!(
            (token.start_frame(), token.order()),
            (mirror.start_frame(), mirror.order()),
            "the two allocators diverged allocating {request} frames"
        );
        addresses[index] = (mirror.start_frame(), mirror.order());
        tokens[index] = Some(token);
    }

    for index in (0..requests.len()).rev() {
        // SAFETY: the unique block has no users and is freed once.
        unsafe {
            by_token
                .deallocate(tokens[index].take().expect("test lost a token"))
                .expect("token deallocation failed");
        }
        let (start, order) = addresses[index];
        // SAFETY: the block has its allocation order, no users, and is freed once.
        unsafe {
            by_address.deallocate_at(start, order).expect("address deallocation failed");
        }
        assert_eq!(
            by_address.free_frames(),
            by_token.free_frames(),
            "freeing {order:?}-order block at {start} by address diverged from the token path"
        );
    }

    assert_eq!(by_token.free_frames(), 64, "token path lost capacity");
    assert_eq!(by_address.free_frames(), 64, "address path lost capacity");

    let token_whole = by_token.allocate(count(64)).expect("token path failed to fully coalesce");
    let address_whole =
        by_address.allocate(count(64)).expect("address path failed to fully coalesce");
    assert_eq!(
        address_whole.start_frame(),
        token_whole.start_frame(),
        "the two paths coalesced to different roots"
    );
}

#[test]
fn deallocate_at_addresses_every_root_of_an_unaligned_range() {
    let frame_range = range(3, 13);
    let mut metadata = [usize::MAX; TEST_METADATA_WORDS];
    let mut allocator = allocator(frame_range, &mut metadata);
    let mut starts = [usize::MAX; 10];

    for slot in &mut starts {
        let block = allocator.allocate(count(1)).expect("single-frame allocation failed");
        *slot = block.start_frame();
    }
    assert_eq!(allocator.free_frames(), 0, "all managed frames should be reserved");

    for &start in &starts {
        // SAFETY: each frame was allocated at order zero, has no users, and is freed once.
        unsafe {
            allocator
                .deallocate_at(start, 0)
                .unwrap_or_else(|error| panic!("freeing frame {start} by address failed: {error}"));
        }
    }
    assert_eq!(allocator.free_frames(), 10, "address frees did not restore every root");

    let merged = allocator.allocate(count(4)).expect("an order-2 root did not coalesce");
    assert_eq!(merged.frame_count(), 4, "coalesced block has the wrong extent");
}

#[test]
fn deallocate_at_detects_a_double_free() {
    let mut metadata = [0usize; TEST_METADATA_WORDS];
    let mut allocator = allocator(range(0, 8), &mut metadata);
    let block = allocator.allocate(count(1)).expect("allocation failed");
    let (start, order) = (block.start_frame(), block.order());
    drop(block);

    // SAFETY: the allocation has no users and is freed once.
    unsafe {
        allocator.deallocate_at(start, order).expect("initial deallocation failed");
    }
    // SAFETY: this intentionally violates the no-double-free contract to verify
    // rejection before allocator state changes.
    let error = unsafe {
        allocator.deallocate_at(start, order).expect_err("duplicate address free was accepted")
    };
    assert_eq!(error, DeallocationError::AlreadyFree, "wrong duplicate-free diagnostic");
    assert_eq!(allocator.free_frames(), 8, "rejected double free changed allocator state");
}

#[test]
fn deallocate_at_rejects_frames_it_does_not_manage() {
    let mut metadata = [0usize; TEST_METADATA_WORDS];
    let mut allocator = allocator(range(8, 16), &mut metadata);
    let block = allocator.allocate(count(1)).expect("allocation failed");
    drop(block);

    for &outsider in &[0usize, 7, 16, 100] {
        // SAFETY: these intentionally invalid frame addresses test rejection before
        // allocator state changes.
        let error = unsafe {
            allocator
                .deallocate_at(outsider, 0)
                .expect_err("frame outside the managed range was accepted")
        };
        assert_eq!(
            error,
            DeallocationError::ForeignBlock,
            "wrong diagnostic for unmanaged frame {outsider}"
        );
    }
    assert_eq!(allocator.free_frames(), 7, "rejected frees changed allocator state");
}

#[test]
fn deallocate_at_rejects_a_start_that_cannot_begin_that_order() {
    let mut metadata = [0usize; TEST_METADATA_WORDS];
    let mut allocator = allocator(range(0, 64), &mut metadata);
    let block = allocator.allocate(count(2)).expect("two-frame allocation failed");
    assert_eq!((block.start_frame(), block.order()), (0, 1), "unexpected two-frame allocation");
    drop(block);

    // SAFETY: this intentionally invalid block start tests alignment rejection before
    // allocator state changes.
    let error =
        unsafe { allocator.deallocate_at(1, 1).expect_err("misaligned address free was accepted") };
    assert_eq!(
        error,
        DeallocationError::UnalignedFrame { start: 1, order: 1 },
        "wrong misalignment diagnostic"
    );
    assert_eq!(allocator.free_frames(), 62, "rejected free changed allocator state");
}

#[test]
fn deallocate_at_rejects_an_order_larger_than_its_root() {
    let mut metadata = [usize::MAX; TEST_METADATA_WORDS];
    let mut allocator = allocator(range(3, 13), &mut metadata);

    // SAFETY: this intentionally invalid order tests root-bound rejection before
    // allocator state changes.
    let error =
        unsafe { allocator.deallocate_at(3, 1).expect_err("order beyond the root was accepted") };
    assert_eq!(error, DeallocationError::ForeignBlock, "wrong over-order diagnostic");

    // SAFETY: this intentionally invalid order tests rejection before shifting or
    // allocator state changes.
    let error = unsafe {
        allocator.deallocate_at(4, usize::BITS as usize).expect_err("absurd order was accepted")
    };
    assert_eq!(error, DeallocationError::ForeignBlock, "wrong absurd-order diagnostic");
    assert_eq!(allocator.free_frames(), 10, "rejected frees changed allocator state");
}
