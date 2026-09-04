//! Scheme-independent RV64 page-table entries.
//!
//! Bits 53:10 hold the physical page number; bits 7:0 hold status and
//! permissions.

use bitflags::bitflags;
use core::fmt;
use core::mem::size_of;

use crate::addr::{MemoryAddr, PhysicalAddr, VirtualAddr};
use crate::geometry::{ENTRY_SIZE, PPN_BITS, page_size_at};
use crate::utils::{field, with_field};

const PTE_PPN_SHIFT: usize = 10;
const FLAG_BITS: usize = 8;

bitflags! {
    /// PTE permission and status bits.
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
    pub const PERMS: Self = Self::READ.union(Self::WRITE).union(Self::EXECUTE);

    pub const READ_WRITE: Self = Self::READ.union(Self::WRITE);
    pub const READ_EXECUTE: Self = Self::READ.union(Self::EXECUTE);
    pub const READ_WRITE_EXECUTE: Self = Self::PERMS;

    pub const USER_READ_WRITE: Self = Self::READ_WRITE.union(Self::USER);
    pub const USER_READ_EXECUTE: Self = Self::READ_EXECUTE.union(Self::USER);
    pub const USER_READ_WRITE_EXECUTE: Self = Self::PERMS.union(Self::USER);

    #[inline]
    pub const fn is_leaf(self) -> bool { self.intersects(Self::PERMS) }

    #[inline]
    pub const fn is_legal_leaf(self) -> bool {
        !self.contains(Self::WRITE) || self.contains(Self::READ)
    }

    /// Format permissions as an `rwx` triple.
    pub const fn rwx(self) -> &'static str {
        match (self.contains(Self::READ), self.contains(Self::WRITE), self.contains(Self::EXECUTE))
        {
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
    pub const fn empty() -> Self { Self(0) }

    pub const fn new(bits: usize) -> Self { Self(bits) }

    /// Build a leaf, adding `VALID` and ignoring `paddr`'s page offset.
    pub const fn leaf(paddr: PhysicalAddr, flags: PteFlags) -> Self {
        Self(
            with_field(0, PTE_PPN_SHIFT, PPN_BITS, paddr.ppn())
                | flags.bits()
                | PteFlags::VALID.bits(),
        )
    }

    /// Build a branch with `VALID` set and no R/W/X bits.
    pub const fn branch(paddr: PhysicalAddr) -> Self {
        Self(with_field(0, PTE_PPN_SHIFT, PPN_BITS, paddr.ppn()) | PteFlags::VALID.bits())
    }

    pub const fn bits(self) -> usize { self.0 }

    pub const fn flags(self) -> PteFlags {
        PteFlags::from_bits_truncate(field(self.0, 0, FLAG_BITS))
    }

    pub const fn ppn(self) -> usize { field(self.0, PTE_PPN_SHIFT, PPN_BITS) }

    pub const fn target(self) -> PhysicalAddr { PhysicalAddr::from_ppn(self.ppn()) }

    /// The physical address `vaddr` resolves to through this leaf at `level`.
    #[inline]
    pub fn phys_at(self, vaddr: VirtualAddr, level: usize) -> PhysicalAddr {
        let page = page_size_at(level);
        self.target().align_down(page).add(vaddr.align_offset(page))
    }

    pub const fn is_valid(self) -> bool { self.flags().contains(PteFlags::VALID) }

    pub const fn is_leaf(self) -> bool { self.is_valid() && self.flags().is_leaf() }

    pub const fn is_branch(self) -> bool { self.is_valid() && !self.flags().is_leaf() }
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
        assert_eq!(
            (PteFlags::READ_EXECUTE | PteFlags::ACCESS | PteFlags::DIRTY | PteFlags::USER).rwx(),
            "r-x",
            "only R/W/X may appear"
        );
    }

    #[test]
    fn entry_validity_and_kind() {
        assert!(!Entry::empty().is_valid(), "zeroed entry is invalid");

        let branch = Entry::branch(PhysicalAddr::new(0x8020_0000));
        assert!(branch.is_branch(), "valid + no perms = branch");
        assert!(!branch.is_leaf());

        let leaf = Entry::leaf(PhysicalAddr::new(0x8020_0000), PteFlags::READ);
        assert!(leaf.is_leaf(), "valid + R = leaf");
        assert!(!leaf.is_branch());

        let stale = Entry::new(PteFlags::READ.bits());
        assert!(!stale.is_leaf(), "invalid entry is never a leaf");
        assert!(!stale.is_branch(), "invalid entry is never a branch");
    }

    #[test]
    fn ppn_storage_preserves_flags() {
        let entry = Entry::leaf(PhysicalAddr::new(0x8020_0ABC), PteFlags::READ_WRITE);

        assert_eq!(entry.ppn(), 0x8020_0000 >> 12, "ppn stored without the page offset");
        assert_eq!(entry.target(), PhysicalAddr::new(0x8020_0000), "target is page-aligned frame");
        assert_eq!(entry.flags(), PteFlags::VALID | PteFlags::READ | PteFlags::WRITE, "flags kept");
    }

    #[test]
    fn the_ppn_field_spans_all_forty_four_bits() {
        let entry = Entry::leaf(PhysicalAddr::from_ppn((1 << PPN_BITS) - 1), PteFlags::READ);

        assert_eq!(entry.ppn(), (1 << PPN_BITS) - 1, "a maximal PPN survives");
        assert_eq!(entry.flags(), PteFlags::VALID | PteFlags::READ, "and does not reach the flags");
    }

    #[test]
    fn a_leaf_resolves_an_address_within_its_page() {
        let leaf = Entry::leaf(PhysicalAddr::new(0x8020_0000), PteFlags::READ);
        assert_eq!(
            leaf.phys_at(VirtualAddr::new(0xDEAD_1ABC), 0),
            PhysicalAddr::new(0x8020_0ABC),
            "a 4 KiB leaf keeps the low 12 bits of the virtual address"
        );

        // A leaf installed mid-superpage still resolves from the superpage base.
        let superpage = Entry::leaf(PhysicalAddr::new(0x8020_1000), PteFlags::READ);
        assert_eq!(
            superpage.phys_at(VirtualAddr::new(0xDEAD_1ABC), 1),
            PhysicalAddr::new(0x802D_1ABC),
            "a 2 MiB leaf keeps the low 21 bits of the virtual address"
        );
    }
}
