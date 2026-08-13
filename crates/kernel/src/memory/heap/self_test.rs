//! Smoke test run immediately after [`super::init`].
//!
//! Its own file for the same reason [`super::super::frame::self_test`] is: test code
//! is not allocator code. A broken global allocator corrupts everything built on it,
//! and the first real consumer is the kernel page table's own region list — so without
//! this, a fault would land during page-table construction, nowhere near its cause.

use paging::sv39::PAGE_SIZE;

use super::{Stats, stats};
use crate::utils::ByteSize;

/// Allocate, reallocate and free through several buddy classes.
pub fn run() {
    let before = stats();

    // A growing `Vec` exercises alloc/copy/dealloc across classes rather than one
    // lucky block.
    let mut counted = alloc::vec::Vec::new();
    for value in 0..1024usize {
        counted.push(value);
    }
    let sum: usize = counted.iter().sum();
    assert_eq!(sum, 1024 * 1023 / 2, "kernel heap self-test: 1024 usizes summed to {sum}");

    // Much larger single block than anything above, so a heap that works only for
    // small requests fails here.
    let block = alloc::boxed::Box::new([0xABu8; PAGE_SIZE]);
    let address = block.as_ptr() as usize;
    assert_eq!(
        address % align_of::<usize>(),
        0,
        "kernel heap self-test: block at {address:#x} is not word aligned"
    );
    assert!(
        block.iter().all(|&byte| byte == 0xAB),
        "kernel heap self-test: a boxed array did not keep its contents"
    );

    drop(counted);
    drop(block);

    let Stats { used, total } = stats();
    assert_eq!(
        used,
        before.used,
        "kernel heap self-test leaked {} bytes",
        used.saturating_sub(before.used)
    );
    // Both numbers: a boot that had to grow during the test says so here.
    println!(
        "[memory] kernel heap self-test passed ({} of {} in use)",
        ByteSize(used),
        ByteSize(total)
    );
}
