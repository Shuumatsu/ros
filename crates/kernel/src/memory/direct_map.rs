//! The kernel's direct-map geometry, and the address conversion it defines.
//!
//! `VA = PA + VA_OFFSET` holds throughout [`DIRECT_MAP_END`] with no RAM base subtracted
//! out, which buys two things: [`phys_to_virt`] is a compile-time add, and it is valid
//! for *every* physical address including MMIO. Skew the offset by the RAM base and
//! `phys_to_virt` of a device address is not even canonical — the linear form is what
//! makes dropping the identity map possible.
//!
//! # How the high half is divided
//!
//! Sv39 gives the kernel one canonical half, and that half is all it will ever have:
//!
//! ```text
//! VA_OFFSET               DIRECT_MAP_END_VA                          usize::MAX
//! |<---- DIRECT_MAP_SPAN ---->|<------- kernel_va, less a guard ------>|
//! |  PA 0..DIRECT_MAP_END     |  addresses the kernel chooses          |
//! ```
//!
//! The split is the point. [`phys_to_virt`] is total over its window, so every address in
//! that window belongs to whichever physical byte it aliases and to nothing else —
//! [`super::kernel_va`] must therefore start where the window ends, not where RAM happens
//! to end. Sizing the window to RAM instead would put a device window above RAM and a
//! chosen kernel address at the same VA, and both mappings would be individually valid.
//!
//! Half the half is the window: 128 GiB of physical reach is more than an Sv39 machine
//! with a 39-bit *virtual* space has any use for, and it leaves an equal amount to hand
//! out. Physical memory or a device window beyond it cannot be aliased at all, so
//! [`super::frame`] drops such RAM and [`super::machine::MachineMemory::check`] rejects
//! such a device.
//!
//! The conversions live next to the constant they are made of; [`super`] re-exports them.
use paging::sv39::{ROOT_ENTRIES_PER_HALF, ROOT_LEVEL, page_size_at};
use paging::{PhysicalAddr, VirtualAddr};

use crate::utils::ByteSize;

/// Bottom of the Sv39 high half, and the base of the kernel's direct map.
///
/// Duplicated in `kernel.ld` as `_va_offset` because the linker cannot read a Rust
/// `const`; [`verify`] keeps the duplicate honest. A bare `usize` because it is a
/// *difference* between the address spaces, not an address in either.
pub const VA_OFFSET: usize = 0xffff_ffc0_0000_0000;

/// Bytes mapped by one root-level leaf.
pub const GIGAPAGE: usize = page_size_at(ROOT_LEVEL);

/// Bytes mapped by one middle-level leaf: the grain the bulk direct map is tiled in, and
/// so the alignment anything placing itself beside it must respect.
pub const SUPERPAGE: usize = page_size_at(1);

/// Bytes of virtual address space in the high half — everything the kernel gets.
const HIGH_HALF: usize = ROOT_ENTRIES_PER_HALF * GIGAPAGE;

// The offset must be the base of that span, or the arithmetic below describes a window
// the hardware places somewhere else.
const _: () = assert!(
    VA_OFFSET.wrapping_add(HIGH_HALF) == 0,
    "VA_OFFSET is not the base of the Sv39 high half"
);

/// Bytes of physical address space the direct map covers, and so how much of the high half
/// it claims. See the module docs for why this is bounded rather than the whole half.
pub const DIRECT_MAP_SPAN: usize = HIGH_HALF / 2;

const _: () = assert!(
    DIRECT_MAP_SPAN % GIGAPAGE == 0 && DIRECT_MAP_SPAN > 0,
    "the direct map is built from gigapages, so its span must be a non-zero multiple of one"
);
const _: () = assert!(
    DIRECT_MAP_SPAN < HIGH_HALF,
    "the direct map must leave the kernel some address space of its own"
);

/// Exclusive end of the physical range the direct map can represent.
///
/// Frames at or above this have no high-half alias, so the frame allocator must not hand
/// them out and no device window may extend past it.
pub const DIRECT_MAP_END: PhysicalAddr = PhysicalAddr::new(DIRECT_MAP_SPAN);

/// First virtual address above the direct map: the bottom of what [`super::kernel_va`]
/// hands out.
///
/// A constant, not a runtime fact — which is what lets `kernel_va` be a plain watermark
/// with no "before the pool exists" state to carry.
pub const DIRECT_MAP_END_VA: VirtualAddr = phys_to_virt(DIRECT_MAP_END);

/// Translate a physical address to its kernel virtual address (`VA = PA + OFFSET`).
///
/// The types are the point of the signature: with bare `usize`s,
/// `phys_to_virt(phys_to_virt(pa))` was a legal expression.
#[inline]
pub const fn phys_to_virt(pa: PhysicalAddr) -> VirtualAddr {
    VirtualAddr::new(pa.bits().wrapping_add(VA_OFFSET))
}

/// Translate a direct-map virtual address back to physical (`PA = VA - OFFSET`).
///
/// Only meaningful for an address *in* the direct map — a secondary's stack VA is not,
/// and the arithmetic cannot tell.
#[inline]
pub const fn virt_to_phys(va: VirtualAddr) -> PhysicalAddr {
    PhysicalAddr::new(va.bits().wrapping_sub(VA_OFFSET))
}

/// Assert the direct map Rust believes in is the one we are running on, against the skew
/// the boot entry measured at its high-half jump. Call once, before the conversions.
pub fn verify(measured: usize) {
    assert_eq!(
        measured, VA_OFFSET,
        "boot entry measured a VA offset of {measured:#x}, but the direct map is built for \
         {VA_OFFSET:#x}; kernel.ld's _va_offset and memory::direct_map::VA_OFFSET have diverged"
    );
}

/// Print the reach of the map every physical address is seen through, and where it stops.
pub fn report() {
    println!(
        "[memory] direct map: PA 0x0..{:#x} -> VA {:#x}..{:#x} ({} addressable)",
        DIRECT_MAP_END,
        VA_OFFSET,
        DIRECT_MAP_END_VA,
        ByteSize(DIRECT_MAP_SPAN)
    );
}
