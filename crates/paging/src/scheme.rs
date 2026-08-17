//! The translation schemes, and the single fact that distinguishes them.
//!
//! Sv39, Sv48 and Sv57 share a page size, a page-table shape, a PTE format and a walk.
//! They differ in how many levels that walk descends — and *only* in that, which is why
//! [`Scheme`] has exactly one required const and derives the rest. Adding a scheme is
//! adding a level count.
//!
//! The types are markers, never values: a scheme is a compile-time choice, so it is a type
//! parameter on [`Mapper`](crate::Mapper) rather than a field in it.

use crate::addr::VirtualAddr;
use crate::geometry::{
    MAX_LEVELS, PAGE_OFFSET_BITS, ROOT_ENTRIES_PER_HALF, VPN_BITS, page_size_at,
};
use crate::satp::Mode;

/// One RV64 translation scheme.
///
/// Everything below [`LEVELS`](Self::LEVELS) is derived and must not be overridden: an
/// implementation that restated one of them would be a second answer to a question
/// `LEVELS` already settles.
pub trait Scheme {
    /// Page-table levels a walk descends. The whole of the difference between schemes.
    const LEVELS: usize;

    /// Index of the root level, where every walk begins.
    const ROOT_LEVEL: usize = Self::LEVELS - 1;

    /// Significant bits in a virtual address: 39, 48 or 57.
    const VA_BITS: usize = PAGE_OFFSET_BITS + VPN_BITS * Self::LEVELS;

    /// The `satp.MODE` encoding that selects this scheme.
    const MODE: Mode = match Mode::from_levels(Self::LEVELS) {
        Some(mode) => mode,
        None => panic!("no satp.MODE encoding walks this many levels"),
    };

    /// Bytes mapped by one root-level leaf: 1 GiB under Sv39, 512 GiB under Sv48.
    const ROOT_PAGE: usize = page_size_at(Self::ROOT_LEVEL);

    /// Bytes of virtual address space in one canonical half — everything a higher-half
    /// kernel gets, and what its direct map is carved out of.
    const HALF_SPAN: usize = ROOT_ENTRIES_PER_HALF * Self::ROOT_PAGE;

    /// True if `va`'s top bits are a correct sign extension of bit `VA_BITS - 1`, i.e. the
    /// address is in the form the hardware accepts.
    fn is_canonical(va: VirtualAddr) -> bool {
        let top = (va.bits() as i64) >> (Self::VA_BITS - 1);
        top == 0 || top == -1
    }

    /// Sign-extend bit `VA_BITS - 1` across the bits above it, producing the canonical
    /// form of `va`.
    fn canonicalize(va: VirtualAddr) -> VirtualAddr {
        let shift = (usize::BITS as usize) - Self::VA_BITS;
        VirtualAddr::new((((va.bits() << shift) as i64) >> shift) as usize)
    }
}

/// 39-bit virtual addresses, three levels, 1 GiB root leaves.
#[derive(Clone, Copy, Debug)]
pub struct Sv39;
impl Scheme for Sv39 {
    const LEVELS: usize = 3;
}

/// 48-bit virtual addresses, four levels, 512 GiB root leaves.
#[derive(Clone, Copy, Debug)]
pub struct Sv48;
impl Scheme for Sv48 {
    const LEVELS: usize = 4;
}

/// 57-bit virtual addresses, five levels, 256 TiB root leaves.
#[derive(Clone, Copy, Debug)]
pub struct Sv57;
impl Scheme for Sv57 {
    const LEVELS: usize = 5;
}

const_assert_eq!(Sv57::LEVELS, MAX_LEVELS);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::{GIGABYTE, TERABYTE};

    #[test]
    fn each_scheme_derives_its_own_geometry_from_the_level_count() {
        assert_eq!(Sv39::VA_BITS, 39, "Sv39 names 39 bits");
        assert_eq!(Sv48::VA_BITS, 48);
        assert_eq!(Sv57::VA_BITS, 57);

        assert_eq!(Sv39::ROOT_LEVEL, 2, "three levels means the root is level 2");
        assert_eq!(Sv57::ROOT_LEVEL, 4);

        assert_eq!(Sv39::MODE, Mode::Sv39, "the mode encoding follows from the level count");
        assert_eq!(Sv48::MODE, Mode::Sv48);
        assert_eq!(Sv57::MODE, Mode::Sv57);

        assert_eq!(Sv39::ROOT_PAGE, GIGABYTE, "an Sv39 root leaf is a gigapage");
        assert_eq!(Sv48::ROOT_PAGE, 512 * GIGABYTE);
    }

    /// The kernel's high half is this, and it is where `direct_map` takes its window from.
    #[test]
    fn a_canonical_half_is_the_root_slots_it_holds() {
        assert_eq!(Sv39::HALF_SPAN, 256 * GIGABYTE, "Sv39 gives a kernel 256 GiB");
        assert_eq!(Sv48::HALF_SPAN, 128 * TERABYTE);
    }

    #[test]
    fn canonicalization_follows_the_scheme_rather_than_a_fixed_width() {
        // Bit 38 is the sign bit under Sv39 and an ordinary address bit under Sv48.
        let va = VirtualAddr::new(1 << 38);
        assert!(!Sv39::is_canonical(va), "bit 38 set is non-canonical under Sv39");
        assert!(Sv48::is_canonical(va), "bit 38 is an ordinary bit under Sv48");

        let fixed = Sv39::canonicalize(va);
        assert!(Sv39::is_canonical(fixed), "canonicalizing produces a canonical address");
        assert_eq!(fixed.vpn(2), va.vpn(2), "canonicalization preserves the VPN fields");
        assert_eq!(Sv48::canonicalize(va), va, "an already-canonical address is unchanged");
    }
}
