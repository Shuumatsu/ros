//! The kernel's direct-map geometry, and the address conversion it defines.
//!
//! `VA = PA + VA_OFFSET` holds throughout [`END`] with no RAM base subtracted out, which
//! buys two things: [`phys_to_virt`] is a compile-time add, and it reaches RAM and MMIO
//! alike. Skew the offset by the RAM base and `phys_to_virt` of a device address is not
//! even canonical — the linear form is what makes dropping the identity map possible.
//!
//! Nothing re-exports the two conversions: a caller writes `direct_map::phys_to_virt`, so
//! the window it is asserting the address lies in is named at the call site. Spelled
//! bare they read as the memory subsystem's answer to "what virtual address is this", and
//! for [`virt_to_phys`] that answer is wrong for every address above [`END`] — where a
//! secondary's stack lives. The general question is [`Mapper::translate`][translate]'s.
//!
//! # How the high half is divided
//!
//! The scheme gives the kernel one canonical half, and that half is all it will ever have:
//!
//! ```text
//! VA_OFFSET                     kernel_va::START                 usize::MAX
//! |<----------- SPAN ---------->|<--- kernel_va, less a guard --->|
//! |  PA 0..END                  |  addresses the kernel chooses   |
//! ```
//!
//! The split is the point. [`phys_to_virt`] is total over its window, so every address in
//! that window belongs to whichever physical byte it aliases and to nothing else —
//! [`super::kernel_va`] must therefore start where the window ends, not where RAM happens
//! to end. Sizing the window to RAM instead would put a device window above RAM and a
//! chosen kernel address at the same VA, and both mappings would be individually valid.
//!
//! Half the half is the window: under Sv39 that is 128 GiB of physical reach, more than a
//! machine with a 39-bit *virtual* space has any use for, and it leaves an equal amount to
//! hand out. Physical memory or a device window beyond it cannot be aliased at all, so
//! [`super::frame`] drops such RAM and [`super::machine::MachineMemory::check`] rejects
//! such a device.
//!
//! [translate]: paging::Mapper::translate
use paging::{PhysicalAddr, Scheme, VirtualAddr};

use super::KernelScheme;
use crate::utils::ByteSize;

/// Bottom of the high half, and the base of the kernel's direct map.
///
/// Duplicated in `kernel.ld` as `_va_offset` because the linker cannot read a Rust
/// `const`; the boot entry reconciles the two before it jumps high. It keeps the linker's
/// spelling rather than shedding the prefix the way [`SPAN`] and [`END`] do, since the two
/// names have to be recognisable as one fact. A bare `usize` because it is a *difference*
/// between the address spaces, not an address in either.
pub const VA_OFFSET: usize = 0xffff_ffc0_0000_0000;

/// Bytes of virtual address space in the high half — everything the kernel gets.
///
/// The scheme's, not a gigapage count: a canonical half is its root slots times whatever
/// one of them covers, which is 1 GiB under Sv39 and 512 GiB under Sv48.
const HIGH_HALF: usize = KernelScheme::HALF_SPAN;

// The offset must be the base of that span, or the arithmetic below describes a window
// the hardware places somewhere else.
const _: () = assert!(
    VA_OFFSET.wrapping_add(HIGH_HALF) == 0,
    "VA_OFFSET is not the base of the high half the kernel's scheme gives it"
);

/// Bytes of physical address space the direct map covers, and so how much of the high half
/// it claims. See the module docs for why this is bounded rather than the whole half.
pub const SPAN: usize = HIGH_HALF / 2;

const _: () = assert!(
    SPAN.is_multiple_of(KernelScheme::ROOT_PAGE) && SPAN > 0,
    "the direct map is built from root-level leaves, so its span must be a non-zero \
     multiple of one"
);
const _: () =
    assert!(SPAN < HIGH_HALF, "the direct map must leave the kernel some address space of its own");

/// Exclusive end of the physical range the direct map can represent.
///
/// Frames at or above this have no high-half alias, so the frame allocator must not hand
/// them out and no device window may extend past it.
pub const END: PhysicalAddr = PhysicalAddr::new(SPAN);

/// Require `[base, base + size)` to lie inside the window, naming it and the constant to
/// raise if it does not.
///
/// The single reach check. A physical range the direct map cannot name is not an error
/// [`phys_to_virt`] can report: it computes an address either way, one outside the window
/// and inside whatever [`super::kernel_va`] hands out next.
///
/// Saturating, so a firmware-supplied size that would wrap cannot produce an end below the
/// base and pass.
pub fn require_reach(what: &str, base: PhysicalAddr, size: usize) {
    let end = base.bits().saturating_add(size);
    assert!(
        end <= END.bits(),
        "{what} at {base:#x}..{end:#x} lies past the direct map's {} window; raise \
         memory::direct_map::SPAN",
        ByteSize(SPAN)
    );
}

/// Translate a physical address to its kernel virtual address (`VA = PA + OFFSET`).
///
/// The types are the point of the signature: with bare `usize`s,
/// `phys_to_virt(phys_to_virt(pa))` is a legal expression.
#[inline]
pub const fn phys_to_virt(pa: PhysicalAddr) -> VirtualAddr {
    VirtualAddr::new(pa.bits().wrapping_add(VA_OFFSET))
}

/// Translate a direct-map virtual address back to physical (`PA = VA - OFFSET`).
///
/// Only meaningful for an address *in* the direct map — a secondary's stack VA is not, and
/// the arithmetic cannot tell. The `debug_assert` can: it is the difference between a
/// wrong answer used as a frame number and a panic naming the address, and it costs a
/// release build nothing.
#[inline]
pub const fn virt_to_phys(va: VirtualAddr) -> PhysicalAddr {
    debug_assert!(
        va.bits() >= VA_OFFSET && va.bits() < VA_OFFSET.wrapping_add(SPAN),
        "virt_to_phys of an address outside the direct map; its physical address is \
         whatever page table maps it, not this subtraction"
    );
    PhysicalAddr::new(va.bits().wrapping_sub(VA_OFFSET))
}

/// Print the reach of the map every physical address is seen through, and where it stops.
pub fn report() {
    println!(
        "[memory] direct map: PA 0x0..{:#x} -> VA {:#x}..{:#x} ({} addressable)",
        END,
        VA_OFFSET,
        phys_to_virt(END),
        ByteSize(SPAN)
    );
}
