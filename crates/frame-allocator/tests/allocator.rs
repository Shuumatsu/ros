use core::num::NonZeroUsize;

use pretty_assertions::assert_eq;

use frame_allocator::{
    DeallocationError, FrameAllocator, FrameBlock, FrameRange, InitError, MetadataError,
    metadata_layout,
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
    // SAFETY: each allocator in these tests is an isolated numeric simulation.
    // Its metadata is separate from the simulated frames, and simultaneously
    // live allocators within a test always manage disjoint ranges.
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

    // `InitError::Metadata` must forward Display to the inner error *and* expose
    // it as the source: the `#[error("{0}")]` + `#[from]` contract. `transparent`
    // would drop the source, so guard it explicitly.
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
}

#[test]
fn reports_exact_metadata_for_unaligned_range() {
    // 3..13 decomposes into aligned roots of 1, 4, 4, and 1 frames.
    // Their buddy trees contain 1 + 7 + 7 + 1 = 16 nodes.
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
    // SAFETY: this isolated simulation owns the numeric range; construction
    // fails before the range can issue any ownership tokens.
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
        // SAFETY: every block is unique, came from this allocator, and is not
        // represented by any live mapping in this host-only test.
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
        // SAFETY: every selected block is unique and has no users.
        unsafe {
            allocator
                .deallocate(blocks[index].take().expect("missing even-index block"))
                .expect("even-index deallocation failed");
        }
    }
    for index in (1..blocks.len()).step_by(2) {
        // SAFETY: every selected block is unique and has no users.
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

    // SAFETY: these distinct blocks came from this allocator and have no users.
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
    // SAFETY: FrameBlock has no destructor. This deliberately duplicates an
    // ownership token to exercise the allocator's unsafe-contract diagnostics.
    let duplicate = unsafe { core::ptr::read(&block) };

    // SAFETY: the first call returns the one live allocation.
    unsafe {
        allocator.deallocate(block).expect("initial deallocation failed");
    }
    // SAFETY: deliberately violating the no-double-free precondition must be
    // diagnosed rather than corrupting the bitmap.
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

    // SAFETY: this intentionally violates allocator provenance to verify that
    // the structural validation rejects the block before changing metadata.
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
        // SAFETY: each unique test block has no users and is returned once.
        unsafe {
            allocator.deallocate(block).expect("fragment deallocation failed");
        }
    }

    assert_eq!(allocator.free_frames(), 64, "fragmented frees did not restore capacity");
    let whole = allocator.allocate(count(64)).expect("full-range allocation failed after merge");
    assert_eq!(whole.start_frame(), 0, "full-range allocation has the wrong base");
}

// ---------------------------------------------------------------------------
// deallocate_at: freeing by address, for callers whose only surviving handle is
// a page-table entry.
// ---------------------------------------------------------------------------

#[test]
fn deallocate_at_round_trips_a_single_frame() {
    let mut metadata = [0usize; TEST_METADATA_WORDS];
    let mut allocator = allocator(range(0, 8), &mut metadata);
    let block = allocator.allocate(count(1)).expect("single-frame allocation failed");
    let (start, order) = (block.start_frame(), block.order());
    assert_eq!(order, 0, "a one-frame request must be order 0");

    // Drop the token unused — the whole point is that the address is enough.
    drop(block);
    assert_eq!(allocator.free_frames(), 7, "allocation accounting is wrong before the free");

    // SAFETY: the frame was just allocated at this exact order, has no users in
    // this host-only numeric test, and is freed once.
    unsafe {
        allocator.deallocate_at(start, order).expect("address-based deallocation failed");
    }
    assert_eq!(allocator.free_frames(), 8, "freeing by address did not restore the frame");

    let whole = allocator.allocate(count(8)).expect("root did not coalesce after an address free");
    assert_eq!(whole.start_frame(), 0, "coalesced root has the wrong base");
}

#[test]
fn deallocate_at_is_indistinguishable_from_token_deallocation() {
    // The load-bearing test: run one allocation sequence on two allocators, free
    // one by token and the other by address, and require the observable state to
    // agree at every step. `deallocate_at` reconstructs a node index, so this is
    // what pins that arithmetic to `allocate`'s.
    let mut token_metadata = [0usize; TEST_METADATA_WORDS];
    let mut address_metadata = [0usize; TEST_METADATA_WORDS];
    let mut by_token = allocator(range(0, 64), &mut token_metadata);
    let mut by_address = allocator(range(0, 64), &mut address_metadata);

    // Deliberately mixed orders, including requests that round up.
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

    // Free in reverse, one step at a time, comparing after each.
    for index in (0..requests.len()).rev() {
        // SAFETY: a unique block from this allocator, freed once, with no users.
        unsafe {
            by_token
                .deallocate(tokens[index].take().expect("test lost a token"))
                .expect("token deallocation failed");
        }
        let (start, order) = addresses[index];
        // SAFETY: same block, same order it was allocated with, freed once.
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

    // Equal counters are not enough — the *shape* of the bitmap must match too,
    // which only a full-range allocation proves.
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
    // 3..13 decomposes into roots of 1, 4, 4, 1 frames at 3, 4, 8 and 12, so this
    // exercises node reconstruction against four different root orders and bit
    // offsets rather than one.
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
        // SAFETY: each frame was allocated at order 0 exactly once and has no users.
        unsafe {
            allocator
                .deallocate_at(start, 0)
                .unwrap_or_else(|error| panic!("freeing frame {start} by address failed: {error}"));
        }
    }
    assert_eq!(allocator.free_frames(), 10, "address frees did not restore every root");

    // Proves the order-2 roots actually coalesced, not merely that a counter moved.
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

    // SAFETY: the one live allocation, freed once.
    unsafe {
        allocator.deallocate_at(start, order).expect("initial deallocation failed");
    }
    // SAFETY: deliberately violating the no-double-free precondition. Freeing by
    // address forfeits the move-only token's compile-time protection, so this must
    // still be caught at run time — by the ancestor that swallowed the block when
    // it coalesced, not by its own bit.
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
        // SAFETY: no frame is actually released; this checks the structural
        // rejection happens before any metadata is touched.
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

    // Frame 1 is inside the block but cannot *begin* an order-1 block. Accepting
    // this would free frames 1..3, straddling two buddies and corrupting the tree.
    // SAFETY: the misuse is the point; it must be rejected, not performed.
    let error = unsafe {
        allocator.deallocate_at(1, 1).expect_err("misaligned address free was accepted")
    };
    assert_eq!(
        error,
        DeallocationError::UnalignedFrame { start: 1, order: 1 },
        "wrong misalignment diagnostic"
    );
    assert_eq!(allocator.free_frames(), 62, "rejected free changed allocator state");
}

#[test]
fn deallocate_at_rejects_an_order_larger_than_its_root() {
    // Frame 3 is a root of its own, of order 0: no larger block can start there.
    let mut metadata = [usize::MAX; TEST_METADATA_WORDS];
    let mut allocator = allocator(range(3, 13), &mut metadata);

    // SAFETY: rejected before any metadata is touched; nothing is released.
    let error = unsafe {
        allocator.deallocate_at(3, 1).expect_err("order beyond the root was accepted")
    };
    assert_eq!(error, DeallocationError::ForeignBlock, "wrong over-order diagnostic");

    // An order that would overflow `1 << order` must be rejected, not shifted.
    // SAFETY: as above.
    let error = unsafe {
        allocator
            .deallocate_at(4, usize::BITS as usize)
            .expect_err("absurd order was accepted")
    };
    assert_eq!(error, DeallocationError::ForeignBlock, "wrong absurd-order diagnostic");
    assert_eq!(allocator.free_frames(), 10, "rejected frees changed allocator state");
}
