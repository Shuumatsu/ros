//! Kernel direct-map geometry and conversions.
//!
//! Half the canonical kernel range is the direct-map window for physical addresses from zero;
//! the other half is reserved for chosen kernel VAs. The fixed split prevents MMIO aliases from
//! colliding with chosen addresses when RAM size changes.
use mmu::{PhysicalAddr, Scheme, VirtualAddr};

use super::KernelScheme;
use crate::utils::ByteSize;

/// Direct-map offset, duplicated as `_va_offset` in `kernel.ld` and verified during boot.
pub const VA_OFFSET: usize = 0xffff_ffc0_0000_0000;

const HIGH_HALF: usize = KernelScheme::HALF_SPAN;

// The offset must begin the scheme's canonical high half.
const _: () = assert!(
    VA_OFFSET.wrapping_add(HIGH_HALF) == 0,
    "VA_OFFSET is not the base of the high half the kernel's scheme gives it"
);

/// Physical bytes covered by the direct map.
pub const SPAN: usize = HIGH_HALF / 2;

const _: () = assert!(
    SPAN.is_multiple_of(KernelScheme::ROOT_PAGE) && SPAN > 0,
    "the direct map is built from root-level leaves, so its span must be a non-zero \
     multiple of one"
);
const _: () =
    assert!(SPAN < HIGH_HALF, "the direct map must leave the kernel some address space of its own");

/// Exclusive end of physical addresses representable by the direct map.
pub const END: PhysicalAddr = PhysicalAddr::new(SPAN);

/// Require `[base, base + size)` to fit in the direct map.
///
/// Saturating end arithmetic rejects overflowing platform ranges.
///
/// # Panics
///
/// Panics if the range exceeds the direct-map window.
pub fn require_reach(what: &str, base: PhysicalAddr, size: usize) {
    let end = base.bits().saturating_add(size);
    assert!(
        end <= END.bits(),
        "{what} at {base:#x}..{end:#x} lies past the direct map's {} window; raise \
         memory::direct_map::SPAN",
        ByteSize(SPAN)
    );
}

/// Translate a physical address with `VA = PA + VA_OFFSET`.
#[inline]
pub const fn phys_to_virt(pa: PhysicalAddr) -> VirtualAddr {
    VirtualAddr::new(pa.bits().wrapping_add(VA_OFFSET))
}

/// Translate a direct-map VA to physical.
///
/// The result is meaningful only inside the direct-map window; debug builds panic otherwise.
#[inline]
pub const fn virt_to_phys(va: VirtualAddr) -> PhysicalAddr {
    debug_assert!(
        va.bits() >= VA_OFFSET && va.bits() < VA_OFFSET.wrapping_add(SPAN),
        "virt_to_phys of an address outside the direct map; its physical address is \
         whatever page table maps it, not this subtraction"
    );
    PhysicalAddr::new(va.bits().wrapping_sub(VA_OFFSET))
}

pub fn report() {
    println!(
        "[memory] direct map: PA 0x0..{:#x} -> VA {:#x}..{:#x} ({} addressable)",
        END,
        VA_OFFSET,
        phys_to_virt(END),
        ByteSize(SPAN)
    );
}
