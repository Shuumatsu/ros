//! The kernel's direct-map geometry, and the address conversion it defines.
//!
//! `VA = PA + VA_OFFSET` holds throughout [`DIRECT_MAP_END`] with no RAM base subtracted
//! out, which buys two things: [`phys_to_virt`] is a compile-time add, and it is valid
//! for *every* physical address including MMIO. Skew the offset by the RAM base and
//! `phys_to_virt` of a device address is not even canonical — the linear form is what
//! makes dropping the identity map possible.
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
const GIGAPAGE: usize = page_size_at(ROOT_LEVEL);

/// Bytes mapped by one middle-level leaf: the grain the bulk direct map is tiled in, and
/// so the alignment anything placing itself beside it must respect.
pub const SUPERPAGE: usize = page_size_at(1);

/// Exclusive end of the physical range the Sv39 direct map can represent.
///
/// One root slot per gigapage and no more, so frames at or above this have no high-half
/// alias and the frame allocator must not hand them out.
pub const DIRECT_MAP_END: PhysicalAddr = PhysicalAddr::new(ROOT_ENTRIES_PER_HALF * GIGAPAGE);

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

/// Print the reach of the map every physical address is seen through — the boot table's
/// blanket mapping, and the ceiling on the real one.
pub fn report() {
    println!(
        "[memory] direct map: PA 0x0..{:#x} -> VA {:#x}.. ({} addressable)",
        DIRECT_MAP_END,
        VA_OFFSET,
        ByteSize(DIRECT_MAP_END.bits())
    );
}
