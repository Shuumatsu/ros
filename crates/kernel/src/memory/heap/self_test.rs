use mmu::PAGE_SIZE;

use super::{Stats, stats};
use crate::utils::ByteSize;

pub fn run() {
    let before = stats();

    let mut counted = alloc::vec::Vec::new();
    for value in 0..1024usize {
        counted.push(value);
    }
    let sum: usize = counted.iter().sum();
    assert_eq!(sum, 1024 * 1023 / 2, "kernel heap self-test: 1024 usizes summed to {sum}");

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
    println!(
        "[memory] kernel heap self-test passed ({} of {} in use)",
        ByteSize(used),
        ByteSize(total)
    );
}
