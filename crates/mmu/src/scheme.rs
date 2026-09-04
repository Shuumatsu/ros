//! RV64 translation-scheme geometry derived from a level count.

use crate::addr::VirtualAddr;
use crate::geometry::{
    MAX_LEVELS, PAGE_OFFSET_BITS, ROOT_ENTRIES_PER_HALF, VPN_BITS, page_size_at,
};
use crate::satp::Mode;
use crate::utils::field;

/// An RV64 translation scheme.
pub trait Scheme {
    /// Number of page-table levels.
    const LEVELS: usize;

    const ROOT_LEVEL: usize = Self::LEVELS - 1;

    const VA_BITS: usize = PAGE_OFFSET_BITS + VPN_BITS * Self::LEVELS;

    const MODE: Mode = match Mode::from_levels(Self::LEVELS) {
        Some(mode) => mode,
        None => panic!("no satp.MODE encoding walks this many levels"),
    };

    const ROOT_PAGE: usize = page_size_at(Self::ROOT_LEVEL);

    const HALF_SPAN: usize = ROOT_ENTRIES_PER_HALF * Self::ROOT_PAGE;

    /// Whether bits above `VA_BITS - 1` sign-extend that bit.
    fn is_canonical(va: VirtualAddr) -> bool { Self::canonicalize(va) == va }

    /// Sign-extend bit `VA_BITS - 1`.
    fn canonicalize(va: VirtualAddr) -> VirtualAddr {
        let shift = (usize::BITS as usize) - Self::VA_BITS;
        VirtualAddr::new((((va.bits() << shift) as i64) >> shift) as usize)
    }
}

/// The 9-bit page-table index `va` uses at `level` under `S` (0 = leaf level).
///
/// `level` must be below `S::LEVELS`.
#[inline]
pub const fn vpn<S: Scheme>(va: VirtualAddr, level: usize) -> usize {
    debug_assert!(level < S::LEVELS, "vpn level out of range for this scheme");
    field(va.bits(), PAGE_OFFSET_BITS + VPN_BITS * level, VPN_BITS)
}

#[derive(Clone, Copy, Debug)]
pub struct Sv39;
impl Scheme for Sv39 {
    const LEVELS: usize = 3;
}

#[derive(Clone, Copy, Debug)]
pub struct Sv48;
impl Scheme for Sv48 {
    const LEVELS: usize = 4;
}

#[derive(Clone, Copy, Debug)]
pub struct Sv57;
impl Scheme for Sv57 {
    const LEVELS: usize = 5;
}

const_assert_eq!(Sv57::LEVELS, MAX_LEVELS);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::GIGAPAGE;

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

        assert_eq!(Sv39::ROOT_PAGE, GIGAPAGE, "an Sv39 root leaf is a gigapage");
        assert_eq!(Sv48::ROOT_PAGE, 512 * GIGAPAGE);
    }

    #[test]
    fn a_canonical_half_is_the_root_slots_it_holds() {
        assert_eq!(Sv39::HALF_SPAN, 256 * GIGAPAGE, "Sv39 gives a kernel 256 GiB");
        assert_eq!(Sv48::HALF_SPAN, 128 * 1024 * GIGAPAGE, "Sv48 gives it 128 TiB");
    }

    #[test]
    fn vpn_decomposes_the_address_into_nine_bit_fields() {
        let va = VirtualAddr::new(0x8200_1ABC);
        assert_eq!(vpn::<Sv39>(va, 2), 2, "vpn[2]");
        assert_eq!(vpn::<Sv39>(va, 1), 16, "vpn[1]");
        assert_eq!(vpn::<Sv39>(va, 0), 1, "vpn[0]");
    }

    #[test]
    fn a_deeper_scheme_indexes_the_same_fields_and_more() {
        let va = VirtualAddr::new((5 << 39) | (7 << 30));
        assert_eq!(vpn::<Sv48>(va, 3), 5, "vpn[3] exists under Sv48");
        assert_eq!(vpn::<Sv48>(va, 2), 7);
        assert_eq!(vpn::<Sv39>(va, 2), vpn::<Sv48>(va, 2), "the shared field decodes the same");
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "out of range for this scheme")]
    fn a_level_the_scheme_does_not_have_is_not_a_field() {
        let _ = vpn::<Sv39>(VirtualAddr::new(1 << 39), 3);
    }

    #[test]
    fn canonicalization_follows_the_scheme_rather_than_a_fixed_width() {
        let va = VirtualAddr::new(1 << 38);
        assert!(!Sv39::is_canonical(va), "bit 38 set is non-canonical under Sv39");
        assert!(Sv48::is_canonical(va), "bit 38 is an ordinary bit under Sv48");

        let fixed = Sv39::canonicalize(va);
        assert!(Sv39::is_canonical(fixed), "canonicalizing produces a canonical address");
        assert_eq!(
            vpn::<Sv39>(fixed, 2),
            vpn::<Sv39>(va, 2),
            "canonicalization preserves the VPN fields"
        );
        assert_eq!(Sv48::canonicalize(va), va, "an already-canonical address is unchanged");
    }
}
