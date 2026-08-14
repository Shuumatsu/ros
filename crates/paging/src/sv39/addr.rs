//! Virtual and physical address types for Sv39 paging.

use super::{LEVELS, PAGE_OFFSET_BITS, PPN_BITS, PPN_FIELD_BITS, VPN_BITS};
use crate::utils::{align_down, align_offset, align_up, field, with_field};
use core::fmt;

/// Number of significant bits in an Sv39 virtual address.
pub const VA_BITS: usize = PAGE_OFFSET_BITS + VPN_BITS * LEVELS; // 39

/// Common arithmetic over machine-word-sized address types.
///
/// Implemented by both [`VirtualAddr`] and [`PhysicalAddr`] so range and
/// alignment logic can be written once and reused for either.
pub trait MemoryAddr: Copy + Clone + Ord + Eq {
    fn from_usize(addr: usize) -> Self;
    fn as_usize(self) -> usize;

    // Alignment
    fn align_down(self, align: usize) -> Self {
        Self::from_usize(align_down(self.as_usize(), align))
    }
    fn align_up(self, align: usize) -> Self { Self::from_usize(align_up(self.as_usize(), align)) }
    fn align_offset(self, align: usize) -> usize { align_offset(self.as_usize(), align) }
    fn is_aligned(self, align: usize) -> bool {
        debug_assert!(align.is_power_of_two(), "alignment must be power of 2");
        self.as_usize() & (align - 1) == 0
    }

    // Signed offset
    fn offset(self, off: isize) -> Self {
        Self::from_usize(self.as_usize().checked_add_signed(off).expect("address overflow"))
    }
    fn wrapping_offset(self, off: isize) -> Self {
        Self::from_usize(self.as_usize().wrapping_add_signed(off))
    }
    fn offset_from(self, base: Self) -> isize {
        (self.as_usize() as isize).checked_sub(base.as_usize() as isize).expect("offset overflow")
    }

    // Addition
    fn add(self, rhs: usize) -> Self {
        Self::from_usize(self.as_usize().checked_add(rhs).expect("address overflow"))
    }
    fn wrapping_add(self, rhs: usize) -> Self {
        Self::from_usize(self.as_usize().wrapping_add(rhs))
    }
    fn checked_add(self, rhs: usize) -> Option<Self> {
        self.as_usize().checked_add(rhs).map(Self::from_usize)
    }

    // Subtraction
    fn sub(self, rhs: usize) -> Self {
        Self::from_usize(self.as_usize().checked_sub(rhs).expect("address underflow"))
    }
    fn wrapping_sub(self, rhs: usize) -> Self {
        Self::from_usize(self.as_usize().wrapping_sub(rhs))
    }
    fn checked_sub(self, rhs: usize) -> Option<Self> {
        self.as_usize().checked_sub(rhs).map(Self::from_usize)
    }

    // Address-to-address distance
    fn sub_addr(self, rhs: Self) -> usize {
        self.as_usize().checked_sub(rhs.as_usize()).expect("address underflow")
    }
    fn checked_sub_addr(self, rhs: Self) -> Option<usize> {
        self.as_usize().checked_sub(rhs.as_usize())
    }
}

/// A 39-bit Sv39 virtual address.
#[repr(transparent)]
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct VirtualAddr(usize);

impl fmt::Debug for VirtualAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "VirtualAddr({:#x}: vpn[2]={}, vpn[1]={}, vpn[0]={}, offset={:#x})",
            self.0,
            self.vpn(2),
            self.vpn(1),
            self.vpn(0),
            self.offset()
        )
    }
}

/// Formats as the bare address, so `{:#x}` works and width flags are honoured.
///
/// Not a nicety. Without it the only way to print an address is `{:?}`, which
/// spells out the VPN decomposition, or `.bits()` — and a caller that reaches for
/// `.bits()` to print has just dropped the type for the rest of the expression.
impl fmt::LowerHex for VirtualAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { fmt::LowerHex::fmt(&self.0, f) }
}

impl MemoryAddr for VirtualAddr {
    fn from_usize(addr: usize) -> Self { Self(addr) }
    fn as_usize(self) -> usize { self.0 }
}

impl VirtualAddr {
    pub const fn new(vaddr: usize) -> Self { Self(vaddr) }

    /// Build an address from a combined virtual page number and a byte offset.
    pub fn from_parts(vpn: usize, offset: usize) -> Self {
        let bits = with_field(0, PAGE_OFFSET_BITS, VPN_BITS * LEVELS, vpn);
        Self(with_field(bits, 0, PAGE_OFFSET_BITS, offset))
    }

    pub const fn bits(self) -> usize { self.0 }

    /// The 9-bit page-table index for `level` (0 = leaf level, 2 = root).
    pub const fn vpn(self, level: usize) -> usize {
        debug_assert!(level < LEVELS, "vpn level out of range");
        field(self.0, PAGE_OFFSET_BITS + VPN_BITS * level, VPN_BITS)
    }

    pub const fn offset(self) -> usize { field(self.0, 0, PAGE_OFFSET_BITS) }

    /// True if bits [63:38] are a correct sign extension of bit 38, i.e. the
    /// address is in the form the hardware accepts.
    pub const fn is_canonical(self) -> bool {
        let top = (self.0 as i64) >> (VA_BITS - 1);
        top == 0 || top == -1
    }

    /// Sign-extend bit 38 across bits [63:39] to produce the canonical form.
    pub const fn canonicalize(self) -> Self {
        let shift = (usize::BITS as usize) - VA_BITS;
        Self((((self.0 << shift) as i64) >> shift) as usize)
    }

    pub const fn as_ptr<T>(self) -> *const T { self.0 as *const T }
    pub const fn as_mut_ptr<T>(self) -> *mut T { self.0 as *mut T }
    pub fn from_ptr_of<T>(ptr: *const T) -> Self { Self::new(ptr as usize) }
    pub fn from_mut_ptr_of<T>(ptr: *mut T) -> Self { Self::new(ptr as usize) }
}

/// A 56-bit Sv39 physical address.
#[repr(transparent)]
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct PhysicalAddr(usize);

impl fmt::Debug for PhysicalAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PhysicalAddr({:#x}, ppn={:#x}, offset={:#x})", self.0, self.ppn(), self.offset())
    }
}

/// See [`VirtualAddr`]'s impl: `{:#x}` without stripping the type.
impl fmt::LowerHex for PhysicalAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { fmt::LowerHex::fmt(&self.0, f) }
}

impl MemoryAddr for PhysicalAddr {
    fn from_usize(addr: usize) -> Self { Self(addr) }
    fn as_usize(self) -> usize { self.0 }
}

impl PhysicalAddr {
    pub const fn new(paddr: usize) -> Self { Self(paddr) }

    /// Build an address from a combined physical page number and a byte offset.
    pub fn from_parts(ppn: usize, offset: usize) -> Self {
        let bits = with_field(0, PAGE_OFFSET_BITS, PPN_BITS, ppn);
        Self(with_field(bits, 0, PAGE_OFFSET_BITS, offset))
    }

    /// The page-aligned physical address of the frame numbered `ppn`.
    pub const fn from_ppn(ppn: usize) -> Self { Self(ppn << PAGE_OFFSET_BITS) }

    pub const fn bits(self) -> usize { self.0 }

    /// The full 44-bit physical page number (`addr >> 12`).
    pub const fn ppn(self) -> usize { field(self.0, PAGE_OFFSET_BITS, PPN_BITS) }

    /// One `PPN[level]` sub-field (level 2 is 26 bits wide, others 9).
    pub const fn ppn_field(self, level: usize) -> usize {
        debug_assert!(level < LEVELS, "ppn level out of range");
        field(self.0, PAGE_OFFSET_BITS + VPN_BITS * level, PPN_FIELD_BITS[level])
    }

    pub const fn offset(self) -> usize { field(self.0, 0, PAGE_OFFSET_BITS) }

    pub const fn as_ptr<T>(self) -> *const T { self.0 as *const T }
    pub const fn as_mut_ptr<T>(self) -> *mut T { self.0 as *mut T }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sv39::PAGE_SIZE;

    mod virtual_addr {
        use super::*;

        #[test]
        fn vpn_and_offset_extraction() {
            // 0x8200_1ABC → vpn2=2, vpn1=16, vpn0=1, offset=0xABC
            let va = VirtualAddr::new(0x8200_1ABC);
            assert_eq!(va.vpn(2), 2, "vpn[2]");
            assert_eq!(va.vpn(1), 16, "vpn[1]");
            assert_eq!(va.vpn(0), 1, "vpn[0]");
            assert_eq!(va.offset(), 0xABC, "offset");
        }

        #[test]
        fn from_parts_roundtrip() {
            let vpn = (0x1FF << 18) | (0x1FF << 9) | 0x1FF; // all fields maxed
            let va = VirtualAddr::from_parts(vpn, 0xFFF);
            assert_eq!(va.vpn(2), 0x1FF);
            assert_eq!(va.vpn(1), 0x1FF);
            assert_eq!(va.vpn(0), 0x1FF);
            assert_eq!(va.offset(), 0xFFF);
        }

        #[test]
        fn alignment() {
            assert!(VirtualAddr::new(0x8000_0000).is_aligned(PAGE_SIZE), "page-aligned");
            assert!(!VirtualAddr::new(0x8000_0001).is_aligned(PAGE_SIZE), "not page-aligned");
            assert!(VirtualAddr::new(0x8020_0000).is_aligned(2 * 1024 * 1024), "2 MiB aligned");
        }

        #[test]
        fn canonicalization() {
            // Low half of the space: already canonical, unchanged.
            let low = VirtualAddr::new(0x8000_0000);
            assert!(low.is_canonical(), "low address is canonical");
            assert_eq!(low.canonicalize().bits(), low.bits(), "low address unchanged");

            // Bit 38 set → canonical form fills the top bits with ones.
            let high = VirtualAddr::new(1 << (VA_BITS - 1));
            assert!(!high.is_canonical(), "raw high-bit address is non-canonical");
            let c = high.canonicalize();
            assert!(c.is_canonical(), "canonicalized address is canonical");
            assert_eq!(c.vpn(2), high.vpn(2), "canonicalization preserves the VPN fields");
        }
    }

    mod physical_addr {
        use super::*;

        #[test]
        fn ppn_extraction() {
            let pa = PhysicalAddr::new(0x8200_1ABC);
            assert_eq!(pa.ppn_field(2), 2, "PPN[2]");
            assert_eq!(pa.ppn_field(1), 16, "PPN[1]");
            assert_eq!(pa.ppn_field(0), 1, "PPN[0]");
            assert_eq!(pa.offset(), 0xABC, "offset");
            assert_eq!(pa.ppn(), 0x8200_1ABC >> 12, "full PPN");
        }

        #[test]
        fn from_parts_and_from_ppn() {
            let pa = PhysicalAddr::from_parts(0x80201, 0xABC);
            assert_eq!(pa.ppn(), 0x80201);
            assert_eq!(pa.offset(), 0xABC);
            assert_eq!(pa.bits(), (0x80201 << 12) | 0xABC);
            assert_eq!(
                PhysicalAddr::from_ppn(0x80201).bits(),
                0x80201 << 12,
                "from_ppn is offset 0"
            );
        }

        #[test]
        fn full_ppn_roundtrip() {
            for ppn in [0usize, 1, 0x80201, (1 << 44) - 1] {
                let pa = PhysicalAddr::from_parts(ppn, 0xFFF);
                assert_eq!(pa.ppn(), ppn, "ppn={ppn:#x}");
                assert_eq!(pa.offset(), 0xFFF, "offset preserved for ppn={ppn:#x}");
            }
        }
    }
}
