//! The kernel's virtual address space above the direct map.
//!
//! Everything the kernel maps at an address of its own choosing takes it from here. One
//! watermark in one module, so two subsystems cannot independently claim the same range —
//! a bug no page-table audit can catch, since both mappings are individually valid.
//!
//! [`START`] is where the direct map stops, which is a constant rather than a function of
//! how much RAM the machine has: these addresses must not alias a physical address, and
//! `phys_to_virt` is total over the direct map's window. [`super::direct_map`] owns that
//! boundary and explains the division.
//!
//! Bump-only: every consumer so far is permanent, and a watermark cannot fragment or
//! double-vend. Something temporary would grow a free list here, and here only.

use core::sync::atomic::{AtomicUsize, Ordering};

use paging::sv39::PAGE_SIZE;
use paging::{MemoryAddr, VirtualAddr};

use crate::memory::direct_map::{DIRECT_MAP_END_VA, GIGAPAGE, SUPERPAGE};
use crate::utils::ByteSize;

/// First virtual address this module hands out: one past the direct map's window.
pub const START: VirtualAddr = DIRECT_MAP_END_VA;

// Finer mappings go here, so the region must begin on a boundary that gives them page-table
// slots of their own rather than sharing one with the direct map's superpages.
const _: () =
    assert!(START.bits() % SUPERPAGE == 0, "the kernel VA region must start superpage-aligned");

/// Watermark, as raw bits: everything from [`START`] up to here has been handed out.
static NEXT: AtomicUsize = AtomicUsize::new(START.bits());

/// Everything from [`START`] up to here has been handed out.
pub fn watermark() -> VirtualAddr { VirtualAddr::new(NEXT.load(Ordering::Relaxed)) }

/// Exclusive end of what this module hands out: the top of the address space, less one
/// [`SUPERPAGE`].
///
/// That superpage is withheld because walking off the end of a mapping there wraps to
/// zero — a *user* address in this kernel. One unmapped superpage turns a kernel pointer
/// silently becoming a user pointer into a fault.
pub const fn end() -> VirtualAddr { VirtualAddr::new(usize::MAX - (SUPERPAGE - 1)) }

/// Bytes of kernel virtual address space still available.
fn remaining() -> usize { end().sub_addr(watermark()) }

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
        let from = watermark();

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
            NEXT.compare_exchange(from.bits(), top.bits(), Ordering::Relaxed, Ordering::Relaxed);
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
    va >= START && va.checked_add(len).is_some_and(|end| end <= watermark())
}

/// Print what is taken and what is left.
pub fn report() {
    // An interval, not a byte count: the remainder is hundreds of gigabytes and
    // page-granular, which `ByteSize` renders exactly and unreadably.
    println!(
        "[memory] kernel VA: {:#x}..{:#x} taken ({}), free through {:#x}",
        START,
        watermark(),
        ByteSize(watermark().sub_addr(START)),
        end()
    );
    if remaining() < GIGAPAGE {
        println!("[memory] WARNING: less than {} of kernel VA space left", ByteSize(GIGAPAGE));
    }
}
