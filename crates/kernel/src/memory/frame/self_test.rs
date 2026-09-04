use mmu::MemoryAddr;
use mmu::PAGE_SIZE;

use super::{alloc, alloc_contiguous, free};
use crate::memory::direct_map::phys_to_virt;

pub fn run() {
    let first = alloc().expect("frame self-test: pool empty on first alloc");
    let first_base = first.base();
    assert!(first_base.is_aligned(PAGE_SIZE), "frame {first_base:?} is not page aligned");
    let first_va = phys_to_virt(first_base);
    // SAFETY: `first` is exclusively owned and writable through the direct map.
    let first_byte = unsafe { core::ptr::read_volatile(first_va.as_ptr::<u8>()) };
    assert_eq!(first_byte, 0u8, "frame {first_base:?} was not zeroed on alloc");

    let second = alloc().expect("frame self-test: pool empty on second alloc");
    assert!(first_base != second.base(), "frame allocator handed out {first_base:?} twice");

    // SAFETY: `first` is exclusively owned and writable through the direct map.
    unsafe { core::ptr::write_bytes(first_va.as_mut_ptr::<u8>(), 0xAB, PAGE_SIZE) };
    // SAFETY: `first` has no remaining users.
    unsafe { free(first) };
    let recycled = alloc().expect("frame self-test: pool empty on realloc");
    if recycled.base() == first_base {
        // SAFETY: `recycled` is exclusively owned and readable through the direct map.
        let byte =
            unsafe { core::ptr::read_volatile(phys_to_virt(recycled.base()).as_ptr::<u8>()) };
        assert_eq!(
            byte,
            0u8,
            "recycled frame {:?} was not re-zeroed (found {byte:#x})",
            recycled.base()
        );
    }

    let run = alloc_contiguous(2).expect("frame self-test: 2-frame contiguous alloc failed");
    assert!(run.base().is_aligned(2 * PAGE_SIZE), "2-frame run {:?} not 8 KiB aligned", run.base());
    assert_eq!(run.bytes(), 2 * PAGE_SIZE, "a 2-frame run must report 8 KiB");

    // SAFETY: each token is unique, from this allocator, and has no users.
    unsafe {
        free(second);
        free(recycled);
        free(run);
    }

    println!("[memory] frame allocator self-test passed");
}
