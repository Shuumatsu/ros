//! Bump allocation for kernel virtual addresses above the direct map.

use core::sync::atomic::{AtomicUsize, Ordering};

use mmu::{MemoryAddr, PAGE_SIZE, SUPERPAGE, VirtualAddr};

use super::direct_map;
use crate::utils::{ByteSize, GIB};

/// First chosen kernel virtual address.
pub const START: VirtualAddr = direct_map::phys_to_virt(direct_map::END);

// The region begins in page-table slots distinct from the direct map's superpages.
const _: () = assert!(
    START.bits().is_multiple_of(SUPERPAGE),
    "the kernel VA region must start superpage-aligned"
);

static NEXT: AtomicUsize = AtomicUsize::new(START.bits());

pub fn watermark() -> VirtualAddr { VirtualAddr::new(NEXT.load(Ordering::Relaxed)) }

/// Exclusive allocation limit, leaving an unmapped superpage before address wraparound.
pub const END: VirtualAddr = VirtualAddr::new(usize::MAX - (SUPERPAGE - 1));

fn remaining() -> usize { END.sub_addr(watermark()) }

/// Reserve `len` page-granular bytes at a multiple of `align`.
///
/// Callers include guard pages in `len`.
///
/// # Panics
///
/// Panics for invalid page geometry or exhaustion.
pub fn reserve(len: usize, align: usize) -> VirtualAddr {
    assert!(align.is_power_of_two(), "kernel VA alignment {align:#x} is not a power of two");
    assert!(align >= PAGE_SIZE, "kernel VA alignment {align:#x} is finer than a page");
    assert!(len > 0, "a kernel VA reservation of nothing has no address to return");
    assert!(
        len.is_multiple_of(PAGE_SIZE),
        "kernel VA reservation of {len:#x} bytes is not a whole number of pages"
    );

    // Compare-exchange prevents concurrent callers from receiving the same range.
    loop {
        let from = watermark();

        let base = from.align_up(align);
        assert!(base >= from, "aligning the kernel VA watermark to {align:#x} wrapped");
        let Some(top) = base.checked_add(len).filter(|top| *top <= END) else {
            panic!(
                "out of kernel virtual address space: {len:#x} bytes wanted at {base:#x}, \
                 {:#x} left below {END:#x}",
                remaining()
            )
        };

        // Relaxed is sufficient because only address ownership is published.
        let claimed =
            NEXT.compare_exchange(from.bits(), top.bits(), Ordering::Relaxed, Ordering::Relaxed);
        if claimed.is_ok() {
            return base;
        }
    }
}

pub fn is_reserved(va: VirtualAddr, len: usize) -> bool {
    va >= START && va.checked_add(len).is_some_and(|end| end <= watermark())
}

const LOW_WATER: usize = GIB;

pub fn report() {
    println!(
        "[memory] kernel VA: {:#x}..{:#x} taken ({}), free through {END:#x}",
        START,
        watermark(),
        ByteSize(watermark().sub_addr(START))
    );
    if remaining() < LOW_WATER {
        println!("[memory] WARNING: less than {} of kernel VA space left", ByteSize(LOW_WATER));
    }
}
