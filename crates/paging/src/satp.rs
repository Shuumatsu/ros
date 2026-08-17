//! The `satp` CSR: which page table is live, and in what translation mode.
//!
//! `satp` is where a root table stops being data and becomes *the* translation.
//! Modelling it here is what keeps the mode encoding out of assembly: the bit
//! layout of the register belongs with the page tables it points at, not
//! duplicated in whatever boot code happens to write it.
//!
//! The RV64 layout is fixed by the privileged spec and does not vary with mode:
//!
//! ```text
//!   | MODE 63:60 | ASID 59:44 | PPN 43:0 |
//! ```
//!
//! So this module sits *above* the schemes rather than inside one — `Sv39` is one of the
//! values [`Mode`] can hold, not a property of the register. The geometry it is pinned
//! against is the crate's, shared by every scheme, for the same reason.

use crate::addr::PhysicalAddr;
use crate::geometry::{PAGE_SIZE, PPN_BITS};
use crate::utils::{field, with_field};

/// Position and width of `satp.PPN` — the root table's physical page number.
const PPN_SHIFT: usize = 0;
/// Position and width of `satp.ASID` — the address-space identifier.
const ASID_SHIFT: usize = 44;
const ASID_BITS: usize = 16;
/// Position and width of `satp.MODE` — the translation scheme.
const MODE_SHIFT: usize = 60;
const MODE_BITS: usize = 4;

// The three fields tile the register exactly, and `PPN` is precisely as wide as
// a physical page number. If `sv39` ever revises `PPN_BITS`, this breaks loudly
// here instead of silently truncating a root address.
const_assert_eq!(PPN_SHIFT, 0);
const_assert_eq!(ASID_SHIFT, PPN_BITS);
const_assert_eq!(MODE_SHIFT, ASID_SHIFT + ASID_BITS);
const_assert_eq!(MODE_SHIFT + MODE_BITS, usize::BITS as usize);

/// Translation-mode encodings for `satp.MODE` on RV64.
///
/// Only the schemes the spec defines; the remaining encodings are reserved, so
/// [`Mode::from_bits`] rejects them rather than inventing a variant.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(usize)]
pub enum Mode {
    /// No translation or protection: addresses are physical.
    Bare = 0,
    /// 39-bit virtual addresses, three levels of page table.
    Sv39 = 8,
    /// 48-bit virtual addresses, four levels.
    Sv48 = 9,
    /// 57-bit virtual addresses, five levels.
    Sv57 = 10,
}

impl Mode {
    /// Every encoding, so the lookups below can search rather than restate.
    const ALL: [Self; 4] = [Self::Bare, Self::Sv39, Self::Sv48, Self::Sv57];

    /// The raw `MODE` field value.
    pub const fn bits(self) -> usize { self as usize }

    /// Decode a `MODE` field, or `None` for a reserved encoding.
    pub const fn from_bits(bits: usize) -> Option<Self> {
        match bits {
            0 => Some(Self::Bare),
            8 => Some(Self::Sv39),
            9 => Some(Self::Sv48),
            10 => Some(Self::Sv57),
            _ => None,
        }
    }

    /// Number of page-table levels this mode walks.
    pub const fn levels(self) -> usize {
        match self {
            Self::Bare => 0,
            Self::Sv39 => 3,
            Self::Sv48 => 4,
            Self::Sv57 => 5,
        }
    }

    /// The mode that walks `levels` levels, or `None` if no encoding does.
    ///
    /// How [`Scheme::MODE`](crate::Scheme::MODE) is derived, which is what keeps a scheme
    /// from naming its own encoding: the level count already determines it. Searched over
    /// [`levels`](Self::levels) rather than written out inverted, so the mapping between
    /// the two exists once.
    pub const fn from_levels(levels: usize) -> Option<Self> {
        let mut index = 0;
        while index < Self::ALL.len() {
            let mode = Self::ALL[index];
            if mode.levels() == levels {
                return Some(mode);
            }
            index += 1;
        }
        None
    }
}

/// A complete `satp` register value.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct Satp(usize);

impl core::fmt::Debug for Satp {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "Satp({:#x}, mode={:?}, asid={}, root={:?})",
            self.0,
            self.mode(),
            self.asid(),
            self.root()
        )
    }
}

impl Satp {
    /// Compose a `satp` value pointing `mode` translation at the root table in
    /// `root`, under address-space id `asid`.
    ///
    /// `const`, so a boot `satp` can be computed at compile time rather than
    /// assembled by hand.
    ///
    /// # Panics
    ///
    /// If `root` is not page-aligned — the register stores only a page number,
    /// so a misaligned root would silently translate through a different frame
    /// than the caller passed. Also if `asid` overflows its field. In a `const`
    /// context both are compile-time errors.
    pub const fn new(mode: Mode, asid: usize, root: PhysicalAddr) -> Self {
        assert!(
            root.bits() & (PAGE_SIZE - 1) == 0,
            "a satp root table address must be page aligned"
        );
        assert!(asid >> ASID_BITS == 0, "asid does not fit satp.ASID");
        Self(
            with_field(0, MODE_SHIFT, MODE_BITS, mode.bits())
                | with_field(0, ASID_SHIFT, ASID_BITS, asid)
                | with_field(0, PPN_SHIFT, PPN_BITS, root.ppn()),
        )
    }

    /// Sv39 translation through `root`, the common case for this kernel.
    pub const fn sv39(root: PhysicalAddr, asid: usize) -> Self { Self::new(Mode::Sv39, asid, root) }

    /// Translation off: physical addressing.
    pub const fn bare() -> Self { Self(0) }

    /// Wrap a raw register value, e.g. one read back out of the CSR.
    pub const fn from_bits(bits: usize) -> Self { Self(bits) }

    /// The raw value to write to the CSR.
    pub const fn bits(self) -> usize { self.0 }

    /// The translation mode, or `None` if the field holds a reserved encoding.
    pub const fn mode(self) -> Option<Mode> {
        Mode::from_bits(field(self.0, MODE_SHIFT, MODE_BITS))
    }

    /// The address-space identifier.
    pub const fn asid(self) -> usize { field(self.0, ASID_SHIFT, ASID_BITS) }

    /// The root table's physical page number.
    pub const fn ppn(self) -> usize { field(self.0, PPN_SHIFT, PPN_BITS) }

    /// The root table's physical address.
    pub const fn root(self) -> PhysicalAddr { PhysicalAddr::from_ppn(self.ppn()) }

    /// Replace the root page number, keeping mode and ASID.
    ///
    /// This is the piece early boot code cannot compute at compile time: the
    /// root table's physical address is a link-time fact. A `const` template
    /// carries mode and ASID; the loader-visible address is folded in here.
    ///
    /// # Panics
    ///
    /// If `root` is not page-aligned.
    pub const fn with_root(self, root: PhysicalAddr) -> Self {
        assert!(
            root.bits() & (PAGE_SIZE - 1) == 0,
            "a satp root table address must be page aligned"
        );
        Self(with_field(self.0, PPN_SHIFT, PPN_BITS, root.ppn()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::page_size_at;
    use crate::scheme::{Scheme, Sv39};

    #[test]
    fn mode_encodings_match_the_privileged_spec() {
        assert_eq!(Mode::Bare.bits(), 0, "Bare is MODE 0");
        assert_eq!(Mode::Sv39.bits(), 8, "Sv39 is MODE 8");
        assert_eq!(Mode::Sv48.bits(), 9);
        assert_eq!(Mode::Sv57.bits(), 10);

        assert_eq!(Mode::from_bits(8), Some(Mode::Sv39), "MODE 8 round-trips");
        assert_eq!(Mode::from_bits(1), None, "MODE 1 is reserved, not a mode");
        assert_eq!(Mode::from_bits(11), None, "MODE 11 is reserved");

        assert_eq!(Mode::Sv39.levels(), Sv39::LEVELS, "Sv39 walks one level per VPN field");
    }

    /// `Scheme::MODE` is derived through this, so a level count that round-trips is what
    /// makes a scheme's encoding follow from its depth rather than be declared beside it.
    #[test]
    fn a_level_count_names_exactly_one_mode() {
        for mode in Mode::ALL {
            assert_eq!(
                Mode::from_levels(mode.levels()),
                Some(mode),
                "{mode:?} must be recoverable from its own level count"
            );
        }
        assert_eq!(Mode::from_levels(2), None, "no RV64 scheme walks two levels");
        assert_eq!(Mode::from_levels(6), None, "nor six");
    }

    #[test]
    fn sv39_value_matches_the_privileged_spec_layout() {
        let root = PhysicalAddr::new(0x8020_1000);
        let satp = Satp::sv39(root, 0);
        assert_eq!(satp.bits(), (8 << 60) | (0x8020_1000 >> 12), "MODE at bit 60, PPN at bit 0");
        assert_eq!(satp.mode(), Some(Mode::Sv39), "mode decodes back");
        assert_eq!(satp.root(), root, "root decodes back");
        assert_eq!(satp.asid(), 0, "no ASID requested");
    }

    #[test]
    fn fields_do_not_bleed_into_each_other() {
        // A root address using the full 44-bit PPN, plus a full-width ASID.
        let root = PhysicalAddr::from_ppn((1 << PPN_BITS) - 1);
        let satp = Satp::new(Mode::Sv57, (1 << ASID_BITS) - 1, root);

        assert_eq!(satp.ppn(), (1 << PPN_BITS) - 1, "PPN survives a maximal ASID");
        assert_eq!(satp.asid(), (1 << ASID_BITS) - 1, "ASID survives a maximal PPN");
        assert_eq!(satp.mode(), Some(Mode::Sv57), "MODE survives both");
    }

    /// The pattern early boot uses: a `const` template carrying only the mode,
    /// with the root folded in once its physical address is known.
    ///
    /// That the *assembly* folding it in agrees with this is not checked here — the
    /// kernel's `memory::boot_table` owns that recipe and asserts it against these
    /// constructors at compile time, next to the code that uses it.
    #[test]
    fn root_can_be_grafted_onto_a_const_template() {
        const TEMPLATE: Satp = Satp::sv39(PhysicalAddr::new(0), 0);
        assert_eq!(TEMPLATE.bits(), 8 << 60, "a rootless template is mode bits only");

        let root = PhysicalAddr::new(0x8100_0000);
        let filled = TEMPLATE.with_root(root);
        assert_eq!(filled, Satp::sv39(root, 0), "grafting == composing directly");
    }

    #[test]
    fn bare_disables_translation() {
        let satp = Satp::bare();
        assert_eq!(satp.bits(), 0, "Bare is an all-zero register");
        assert_eq!(satp.mode(), Some(Mode::Bare));
        assert_eq!(Satp::from_bits(0), satp, "raw 0 round-trips to Bare");
    }

    #[test]
    #[should_panic(expected = "page aligned")]
    fn rejects_a_misaligned_root() {
        // Silently dropping these low bits is exactly the bug the assert exists
        // to prevent: the walk would follow a different frame than requested.
        let _ = Satp::sv39(PhysicalAddr::new(0x8020_0800), 0);
    }

    #[test]
    #[should_panic(expected = "asid")]
    fn rejects_an_oversized_asid() {
        let _ = Satp::new(Mode::Sv39, 1 << ASID_BITS, PhysicalAddr::new(0));
    }

    /// A root table is one page, so a gigapage root mapping and the satp PPN
    /// agree on what "page number" means.
    #[test]
    fn page_size_agrees_with_the_ppn_shift() {
        assert_eq!(PAGE_SIZE, 4096, "satp.PPN is an address >> 12");
        assert_eq!(page_size_at(Sv39::ROOT_LEVEL), 1 << 30, "an Sv39 root leaf is a gigapage");
    }
}
