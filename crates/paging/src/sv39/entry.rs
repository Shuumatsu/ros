//! Page-table entry type and permission flags for Sv39.

use bitflags::bitflags;
use core::fmt;
use core::mem::size_of;

use super::addr::PhysicalAddr;
use super::{ENTRY_SIZE, PPN_BITS, PPN_FIELD_BITS, VPN_BITS};
use crate::utils::{field, with_field};

/// Bit position at which the PPN begins inside a PTE.
const PTE_PPN_SHIFT: usize = 10;
/// Number of permission/status flag bits at the bottom of a PTE.
const FLAG_BITS: usize = 8;

bitflags! {
    /// The low-8 permission and status bits of a page-table entry.
    ///
    /// A *leaf* entry sets at least one of R/W/X; a *branch* (pointer to the
    /// next level) sets none of them. `W` without `R` is a reserved encoding.
    #[repr(transparent)]
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub struct PteFlags: usize {
        const VALID   = 1 << 0;
        const READ    = 1 << 1;
        const WRITE   = 1 << 2;
        const EXECUTE = 1 << 3;
        const USER    = 1 << 4;
        const GLOBAL  = 1 << 5;
        const ACCESS  = 1 << 6;
        const DIRTY   = 1 << 7;
    }
}

impl PteFlags {
    /// The R/W/X permission bits. Their presence distinguishes leaf from branch.
    pub const PERMS: Self = Self::READ.union(Self::WRITE).union(Self::EXECUTE);

    pub const READ_WRITE: Self = Self::READ.union(Self::WRITE);
    pub const READ_EXECUTE: Self = Self::READ.union(Self::EXECUTE);
    pub const READ_WRITE_EXECUTE: Self = Self::PERMS;

    pub const USER_READ_WRITE: Self = Self::READ_WRITE.union(Self::USER);
    pub const USER_READ_EXECUTE: Self = Self::READ_EXECUTE.union(Self::USER);
    pub const USER_READ_WRITE_EXECUTE: Self = Self::PERMS.union(Self::USER);

    /// True if these flags describe a leaf mapping (any of R/W/X set).
    #[inline]
    pub const fn is_leaf(self) -> bool {
        self.bits() & Self::PERMS.bits() != 0
    }

    /// True if the R/W/X combination is architecturally legal (W implies R).
    #[inline]
    pub const fn is_legal_leaf(self) -> bool {
        self.bits() & Self::WRITE.bits() == 0 || self.bits() & Self::READ.bits() != 0
    }

    /// The permission bits as a conventional `rwx` triple, `-` for absent.
    ///
    /// Lives here because this type owns what R/W/X *mean*; anything printing a
    /// memory map should not be re-deriving the spelling for itself.
    pub const fn rwx(self) -> &'static str {
        match (
            self.bits() & Self::READ.bits() != 0,
            self.bits() & Self::WRITE.bits() != 0,
            self.bits() & Self::EXECUTE.bits() != 0,
        ) {
            (false, false, false) => "---",
            (false, false, true) => "--x",
            (false, true, false) => "-w-",
            (false, true, true) => "-wx",
            (true, false, false) => "r--",
            (true, false, true) => "r-x",
            (true, true, false) => "rw-",
            (true, true, true) => "rwx",
        }
    }
}

/// A single Sv39 page-table entry.
#[derive(Copy, Clone, PartialEq, Eq)]
#[repr(transparent)]
pub struct Entry(usize);
const_assert_eq!(size_of::<Entry>(), ENTRY_SIZE);

impl fmt::Debug for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Entry({:#x}, ppn={:#x}, {:?})", self.0, self.ppn(), self.flags())
    }
}

impl Entry {
    /// An empty (invalid, all-zero) entry.
    pub const fn empty() -> Self { Self(0) }

    /// Wrap a raw entry word.
    pub const fn new(bits: usize) -> Self { Self(bits) }

    /// A leaf entry mapping the frame `paddr` lies in, with `flags`; `VALID` is
    /// applied automatically and `paddr`'s offset bits are ignored.
    ///
    /// `const`, so a whole page table can be built at compile time — which is
    /// how the early boot table is made, before any allocator exists.
    pub const fn leaf(paddr: PhysicalAddr, flags: PteFlags) -> Self {
        Self(
            with_field(0, PTE_PPN_SHIFT, PPN_BITS, paddr.ppn())
                | flags.bits()
                | PteFlags::VALID.bits(),
        )
    }

    /// A branch entry pointing at the next-level table in `paddr`.
    ///
    /// Carries no R/W/X, so a stale permission bit can never turn an
    /// intermediate table into an accidental leaf.
    pub const fn branch(paddr: PhysicalAddr) -> Self {
        Self(with_field(0, PTE_PPN_SHIFT, PPN_BITS, paddr.ppn()) | PteFlags::VALID.bits())
    }

    pub const fn bits(self) -> usize { self.0 }

    pub const fn flags(self) -> PteFlags {
        PteFlags::from_bits_truncate(field(self.0, 0, FLAG_BITS))
    }

    pub fn set_flags(&mut self, flags: PteFlags) {
        self.0 = with_field(self.0, 0, FLAG_BITS, flags.bits());
    }

    /// The full 44-bit physical page number this entry carries.
    pub const fn ppn(self) -> usize { field(self.0, PTE_PPN_SHIFT, PPN_BITS) }

    /// One `PPN[level]` sub-field (level 2 is 26 bits wide, others 9).
    pub const fn ppn_field(self, level: usize) -> usize {
        debug_assert!(level < super::LEVELS, "ppn level out of range");
        field(self.0, PTE_PPN_SHIFT + VPN_BITS * level, PPN_FIELD_BITS[level])
    }

    /// Store the physical frame `paddr` points into (its offset is ignored).
    pub fn set_ppn(&mut self, paddr: PhysicalAddr) {
        self.0 = with_field(self.0, PTE_PPN_SHIFT, PPN_BITS, paddr.ppn());
    }

    /// The page-aligned physical address this entry targets.
    pub const fn target(self) -> PhysicalAddr { PhysicalAddr::from_ppn(self.ppn()) }

    pub const fn is_valid(self) -> bool {
        field(self.0, 0, FLAG_BITS) & PteFlags::VALID.bits() != 0
    }

    /// A valid entry that maps a page (has R/W/X).
    pub const fn is_leaf(self) -> bool {
        self.is_valid() && self.flags().is_leaf()
    }

    /// A valid entry that points to a next-level table (no R/W/X).
    pub const fn is_branch(self) -> bool {
        self.is_valid() && !self.flags().is_leaf()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_bit_positions() {
        assert_eq!(PteFlags::VALID.bits(), 1 << 0);
        assert_eq!(PteFlags::READ.bits(), 1 << 1);
        assert_eq!(PteFlags::WRITE.bits(), 1 << 2);
        assert_eq!(PteFlags::EXECUTE.bits(), 1 << 3);
        assert_eq!(PteFlags::USER.bits(), 1 << 4);
        assert_eq!(PteFlags::DIRTY.bits(), 1 << 7);
    }

    #[test]
    fn leaf_vs_branch_flags() {
        assert!(!PteFlags::VALID.is_leaf(), "V-only is a branch");
        assert!(PteFlags::READ.is_leaf(), "R is a leaf");
        assert!(PteFlags::EXECUTE.is_leaf(), "X is a leaf");
        assert!(PteFlags::READ_WRITE.is_legal_leaf(), "RW is legal");
        assert!(!PteFlags::WRITE.is_legal_leaf(), "W-only is reserved");
    }

    #[test]
    fn rwx_spells_out_the_permission_bits() {
        assert_eq!(PteFlags::READ_EXECUTE.rwx(), "r-x", "kernel text");
        assert_eq!(PteFlags::READ_WRITE.rwx(), "rw-", "kernel data");
        assert_eq!(PteFlags::READ.rwx(), "r--", "kernel rodata");
        assert_eq!(PteFlags::READ_WRITE_EXECUTE.rwx(), "rwx", "the boot table's blanket rights");
        assert_eq!(PteFlags::VALID.rwx(), "---", "a branch carries no permissions");
        // Status and USER bits must not leak into the triple.
        assert_eq!(
            (PteFlags::READ_EXECUTE | PteFlags::ACCESS | PteFlags::DIRTY | PteFlags::USER).rwx(),
            "r-x",
            "only R/W/X may appear"
        );
    }

    #[test]
    fn entry_validity_and_kind() {
        assert!(!Entry::empty().is_valid(), "zeroed entry is invalid");

        let mut branch = Entry::empty();
        branch.set_flags(PteFlags::VALID);
        assert!(branch.is_branch(), "valid + no perms = branch");
        assert!(!branch.is_leaf());

        let mut leaf = Entry::empty();
        leaf.set_flags(PteFlags::VALID | PteFlags::READ);
        assert!(leaf.is_leaf(), "valid + R = leaf");
        assert!(!leaf.is_branch());

        // R/W/X set but not valid → neither leaf nor branch (e.g. after unmap).
        let stale = Entry::new(PteFlags::READ.bits());
        assert!(!stale.is_leaf(), "invalid entry is never a leaf");
        assert!(!stale.is_branch(), "invalid entry is never a branch");
    }

    #[test]
    fn ppn_storage_preserves_flags() {
        let mut entry = Entry::empty();
        entry.set_flags(PteFlags::VALID | PteFlags::READ | PteFlags::WRITE);
        entry.set_ppn(PhysicalAddr::new(0x8020_0000));

        assert_eq!(entry.ppn(), 0x8020_0000 >> 12, "ppn stored");
        assert_eq!(entry.target(), PhysicalAddr::new(0x8020_0000), "target is page-aligned frame");
        assert_eq!(entry.flags(), PteFlags::VALID | PteFlags::READ | PteFlags::WRITE, "flags kept");
    }

    #[test]
    fn ppn_field_split() {
        // PPN[2] is 26 bits at PTE bit 28; PPN[0] is 9 bits at bit 10.
        let entry = Entry::new(0x3FF_FFFF << 28);
        assert_eq!(entry.ppn_field(2), 0x3FF_FFFF, "26-bit PPN[2]");
        assert_eq!(entry.ppn_field(0), 0);

        let entry = Entry::new(0x1FF << 10);
        assert_eq!(entry.ppn_field(0), 0x1FF, "9-bit PPN[0]");
        assert_eq!(entry.ppn_field(2), 0);
    }
}
