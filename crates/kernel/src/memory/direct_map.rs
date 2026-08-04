//! The kernel's direct-map geometry.
//!
//! # Why a *linear* map
//!
//! `VA = PA + VA_OFFSET` holds throughout [`DIRECT_MAP_END`], with no RAM base
//! subtracted out, which buys two things:
//!
//! - [`super::phys_to_virt`] is a compile-time add — no runtime offset to record
//!   and no window in which it silently returns garbage because it has not been
//!   recorded yet.
//! - It is valid for *every* physical address in that range, MMIO included — skew
//!   the offset by the RAM base and `phys_to_virt` on a device address yields a
//!   non-canonical Sv39 address. That is what makes eventually dropping the
//!   identity map possible.
//!
use paging::PhysicalAddr;
use paging::sv39::{ENTRIES_PER_PAGE, ROOT_LEVEL, page_size_at};

/// Bottom of the Sv39 high half, and the base of the kernel's direct map.
///
/// Duplicated in `kernel.ld` as `_va_offset` out of necessity — the linker cannot read
/// a Rust `const`. [`verify`] is what keeps the duplicate honest.
///
/// A bare `usize` while its neighbours are typed, because it is a *difference* between
/// the two address spaces rather than an address in either. [`super::phys_to_virt`] is
/// where it becomes one.
pub const VA_OFFSET: usize = 0xffff_ffc0_0000_0000;

/// Bytes mapped by one root-level leaf.
const GIGAPAGE: usize = page_size_at(ROOT_LEVEL);

/// Root-level leaves in either canonical half of the Sv39 address space.
///
/// The low half is the identity map and the high half is the direct map, so every
/// root entry has one fixed role. Filling all of them costs no more memory than a
/// partial table: an Sv39 root is always one 4 KiB page.
pub(crate) const ROOT_ENTRIES: usize = ENTRIES_PER_PAGE / 2;

/// Exclusive end of the physical range representable by the Sv39 direct map.
///
/// Frames at or above this address have no high-half alias, so the frame allocator
/// must not hand them out.
pub const DIRECT_MAP_END: PhysicalAddr = PhysicalAddr::new(ROOT_ENTRIES * GIGAPAGE);

/// Assert the direct map Rust believes in is the one we are actually running on.
///
/// The naked boot entry measures the linked-to-physical skew at its high-half jump.
/// This catches disagreement between the linker script and the Rust constant.
///
/// Call once before using the address conversion helpers.
pub fn verify(measured: usize) {
    assert_eq!(
        measured, VA_OFFSET,
        "boot entry measured a VA offset of {measured:#x}, but the direct map is built for \
         {VA_OFFSET:#x}; kernel.ld's _va_offset and memory::direct_map::VA_OFFSET have diverged"
    );
}
