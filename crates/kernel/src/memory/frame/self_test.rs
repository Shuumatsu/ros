//! Smoke test run immediately after [`super::init`].
//!
//! A broken frame allocator corrupts page tables and everything mapped through them,
//! so this panics loudly and early rather than letting the kernel limp on. Its own
//! file because it is test code, not allocator code, and the properties it pins
//! (aligned, zeroed, distinct, re-zeroed on reuse) are worth reading as a list.

use paging::MemoryAddr;
use paging::sv39::PAGE_SIZE;

use super::{alloc, alloc_contiguous, free};
use crate::memory::direct_map::phys_to_virt;

/// Check the properties every caller of [`super::alloc`] relies on.
pub fn run() {
    // (1) A freshly allocated frame is page aligned and zeroed.
    let first = alloc().expect("frame self-test: pool empty on first alloc");
    let first_base = first.base();
    assert!(first_base.is_aligned(PAGE_SIZE), "frame {first_base:?} is not page aligned");
    let first_va = phys_to_virt(first_base);
    // SAFETY: the allocator just gave us exclusive ownership of this frame, and it is
    // mapped read-write through the direct map.
    let first_byte = unsafe { core::ptr::read_volatile(first_va.as_ptr::<u8>()) };
    assert_eq!(first_byte, 0u8, "frame {first_base:?} was not zeroed on alloc");

    // (2) A second frame is distinct.
    let second = alloc().expect("frame self-test: pool empty on second alloc");
    assert!(first_base != second.base(), "frame allocator handed out {first_base:?} twice");

    // (3) Dirty then free the first; if that frame comes back, it must be re-zeroed —
    //     proving alloc zeroes recycled frames, not just pristine RAM.
    // SAFETY: as above; this frame is ours and has no other users.
    unsafe { core::ptr::write_bytes(first_va.as_mut_ptr::<u8>(), 0xAB, PAGE_SIZE) };
    // SAFETY: `first` has no users in this host-free test.
    unsafe { free(first) };
    let recycled = alloc().expect("frame self-test: pool empty on realloc");
    if recycled.base() == first_base {
        // SAFETY: as above.
        let byte =
            unsafe { core::ptr::read_volatile(phys_to_virt(recycled.base()).as_ptr::<u8>()) };
        assert_eq!(
            byte,
            0u8,
            "recycled frame {:?} was not re-zeroed (found {byte:#x})",
            recycled.base()
        );
    }

    // (4) A 2-frame contiguous run is aligned to its size, and reports its size.
    let run = alloc_contiguous(2).expect("frame self-test: 2-frame contiguous alloc failed");
    assert!(run.base().is_aligned(2 * PAGE_SIZE), "2-frame run {:?} not 8 KiB aligned", run.base());
    assert_eq!(run.bytes(), 2 * PAGE_SIZE, "a 2-frame run must report 8 KiB");

    // Release everything still held (freeing `recycled` covers `first`, which became it).
    // SAFETY: each token is unique, from this allocator, and has no users.
    unsafe {
        free(second);
        free(recycled);
        free(run);
    }

    println!("[memory] frame allocator self-test passed");
}
