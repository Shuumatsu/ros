//! The kernel's virtual address space above the direct map.
//!
//! Everything the kernel maps at an address of its own choosing takes it from here. One
//! watermark in one module, so two subsystems cannot independently claim the same range —
//! a bug no page-table audit can catch, since both mappings are individually valid.
//!
//! [`start`] sits just above the direct map, superpage-aligned so these finer mappings
//! get a page-table slot of their own. It is a *runtime* fact: how much RAM the allocator
//! owns decides how far the direct map reaches. Between it and [`end`] is whatever the
//! direct map did not need of the [`DIRECT_MAP_END`]-wide high half.
//!
//! Bump-only: every consumer so far is permanent, and a watermark cannot fragment or
//! double-vend. Something temporary would grow a free list here, and here only.

use core::sync::atomic::{AtomicUsize, Ordering};

use paging::sv39::PAGE_SIZE;
use paging::{MemoryAddr, VirtualAddr};

use crate::memory::direct_map::{DIRECT_MAP_END, SUPERPAGE};
use crate::memory::{frame, phys_to_virt};
use crate::utils::ByteSize;

/// Watermark, as raw bits. `UNSET` until the first [`reserve`], since [`start`] cannot be
/// evaluated in a `static` initializer.
static NEXT: AtomicUsize = AtomicUsize::new(UNSET);

/// Nothing handed out yet. Zero is safe as the sentinel: every address here is in the
/// high half, and [`reserve`] rejects the wrap needed to produce it.
const UNSET: usize = 0;

/// First virtual address above everything the direct map occupies.
///
/// # Panics
///
/// Before [`super::frame::init`], since the direct map's extent is not known until then.
pub fn start() -> VirtualAddr {
    let (_, pool_end) = frame::owned_range();
    phys_to_virt(pool_end).align_up(SUPERPAGE)
}

/// Everything from [`start`] up to here has been handed out.
pub fn watermark() -> VirtualAddr {
    match NEXT.load(Ordering::Relaxed) {
        UNSET => start(),
        bits => VirtualAddr::new(bits),
    }
}

/// Exclusive end of what this module hands out: the top of the address space, less one
/// [`SUPERPAGE`].
///
/// That superpage is withheld because walking off the end of a mapping there wraps to
/// zero — a *user* address in this kernel. One unmapped superpage turns a kernel pointer
/// silently becoming a user pointer into a fault.
pub fn end() -> VirtualAddr { VirtualAddr::new(usize::MAX).align_down(SUPERPAGE) }

/// Bytes of kernel virtual address space still available.
pub fn remaining() -> usize { end().sub_addr(watermark()) }

/// Take `len` bytes of kernel virtual address space, based at a multiple of `align`.
///
/// Whole pages only, in both arguments: an address from here exists to be mapped at, and
/// a mapping is made of pages. A caller wanting a guard page asks for it as part of
/// `len`; the hole is its business.
///
/// # Panics
///
/// If the request is not page-shaped, or the space is exhausted — a panic rather than an
/// `Option`, since no smaller request would help an allocator that only moves up.
pub fn reserve(len: usize, align: usize) -> VirtualAddr {
    assert!(align.is_power_of_two(), "kernel VA alignment {align:#x} is not a power of two");
    assert!(align >= PAGE_SIZE, "kernel VA alignment {align:#x} is finer than a page");
    assert!(len > 0, "a kernel VA reservation of nothing has no address to return");
    assert!(
        len.is_multiple_of(PAGE_SIZE),
        "kernel VA reservation of {len:#x} bytes is not a whole number of pages"
    );

    // Compare-exchange, though today's only caller is the boot hart: this is the single
    // answer to which addresses are free, and a load/store pair would let a second caller
    // observe the same watermark twice.
    loop {
        let observed = NEXT.load(Ordering::Relaxed);
        let from = if observed == UNSET { start() } else { VirtualAddr::new(observed) };

        let base = from.align_up(align);
        assert!(base >= from, "aligning the kernel VA watermark to {align:#x} wrapped");
        let top = base.checked_add(len).filter(|top| *top <= end()).unwrap_or_else(|| {
            panic!(
                "out of kernel virtual address space: {len:#x} bytes wanted at {base:#x}, \
                 {:#x} left below {:#x}",
                remaining(),
                end()
            )
        });

        // Relaxed: what is published is address space, not data. Nothing is read through
        // one of these addresses until its owner maps it, under the page table's lock.
        let claimed =
            NEXT.compare_exchange(observed, top.bits(), Ordering::Relaxed, Ordering::Relaxed);
        if claimed.is_ok() {
            return base;
        }
    }
}

/// Whether `[va, va + len)` was handed out by this module.
///
/// [`super::kernel_table`] audits every region against this, which is what makes "one
/// owner for kernel VA space" a checked property rather than a convention.
pub fn is_reserved(va: VirtualAddr, len: usize) -> bool {
    va >= start() && va.checked_add(len).is_some_and(|end| end <= watermark())
}

/// Print what is taken and what is left.
pub fn report() {
    // An interval, not a byte count: the remainder is hundreds of gigabytes and
    // page-granular, which `ByteSize` renders exactly and unreadably.
    println!(
        "[memory] kernel VA: {:#x}..{:#x} taken ({}), free through {:#x}",
        start(),
        watermark(),
        ByteSize(watermark().sub_addr(start())),
        end()
    );
    // This and the direct map share one canonical half, so more RAM means less here.
    if remaining() < DIRECT_MAP_END.bits() / 64 {
        println!("[memory] WARNING: kernel virtual address space is nearly exhausted");
    }
}
