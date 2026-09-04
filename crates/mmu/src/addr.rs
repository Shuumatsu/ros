//! Typed virtual and physical addresses.

use crate::geometry::{PAGE_OFFSET_BITS, PPN_BITS};
use crate::utils::{align_down, align_offset, align_up, field, is_aligned, with_field};
use core::fmt;

/// Common checked arithmetic and alignment operations for address types.
pub trait MemoryAddr: Copy + Clone + Ord + Eq {
    fn from_usize(addr: usize) -> Self;
    fn as_usize(self) -> usize;

    fn align_down(self, align: usize) -> Self {
        Self::from_usize(align_down(self.as_usize(), align))
    }
    fn align_up(self, align: usize) -> Self { Self::from_usize(align_up(self.as_usize(), align)) }
    fn align_offset(self, align: usize) -> usize { align_offset(self.as_usize(), align) }
    fn is_aligned(self, align: usize) -> bool { is_aligned(self.as_usize(), align) }

    /// `[self, end)` rounded outward to whole `align`-sized units.
    fn footprint(self, end: Self, align: usize) -> (Self, Self) {
        (self.align_down(align), end.align_up(align))
    }

    fn add(self, rhs: usize) -> Self {
        Self::from_usize(self.as_usize().checked_add(rhs).expect("address overflow"))
    }
    fn checked_add(self, rhs: usize) -> Option<Self> {
        self.as_usize().checked_add(rhs).map(Self::from_usize)
    }

    fn sub(self, rhs: usize) -> Self {
        Self::from_usize(self.as_usize().checked_sub(rhs).expect("address underflow"))
    }

    fn sub_addr(self, rhs: Self) -> usize {
        self.as_usize().checked_sub(rhs.as_usize()).expect("address underflow")
    }
    fn checked_sub_addr(self, rhs: Self) -> Option<usize> {
        self.as_usize().checked_sub(rhs.as_usize())
    }
}

#[repr(transparent)]
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct VirtualAddr(usize);

impl fmt::Debug for VirtualAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "VirtualAddr({:#x})", self.0)
    }
}

impl fmt::LowerHex for VirtualAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { fmt::LowerHex::fmt(&self.0, f) }
}

impl MemoryAddr for VirtualAddr {
    fn from_usize(addr: usize) -> Self { Self(addr) }
    fn as_usize(self) -> usize { self.0 }
}

impl VirtualAddr {
    pub const fn new(vaddr: usize) -> Self { Self(vaddr) }

    pub const fn bits(self) -> usize { self.0 }

    pub const fn offset(self) -> usize { field(self.0, 0, PAGE_OFFSET_BITS) }

    pub const fn as_ptr<T>(self) -> *const T { self.0 as *const T }
    pub const fn as_mut_ptr<T>(self) -> *mut T { self.0 as *mut T }
    pub fn from_ptr_of<T>(ptr: *const T) -> Self { Self::new(ptr as usize) }
    pub fn from_mut_ptr_of<T>(ptr: *mut T) -> Self { Self::new(ptr as usize) }
}

/// A 56-bit RV64 physical address.
#[repr(transparent)]
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct PhysicalAddr(usize);

impl fmt::Debug for PhysicalAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PhysicalAddr({:#x}, ppn={:#x}, offset={:#x})", self.0, self.ppn(), self.offset())
    }
}

impl fmt::LowerHex for PhysicalAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { fmt::LowerHex::fmt(&self.0, f) }
}

impl MemoryAddr for PhysicalAddr {
    fn from_usize(addr: usize) -> Self { Self(addr) }
    fn as_usize(self) -> usize { self.0 }
}

impl PhysicalAddr {
    pub const fn new(paddr: usize) -> Self { Self(paddr) }

    pub const fn from_parts(ppn: usize, offset: usize) -> Self {
        let bits = with_field(0, PAGE_OFFSET_BITS, PPN_BITS, ppn);
        Self(with_field(bits, 0, PAGE_OFFSET_BITS, offset))
    }

    pub const fn from_ppn(ppn: usize) -> Self { Self(ppn << PAGE_OFFSET_BITS) }

    pub const fn bits(self) -> usize { self.0 }

    pub const fn ppn(self) -> usize { field(self.0, PAGE_OFFSET_BITS, PPN_BITS) }

    pub const fn offset(self) -> usize { field(self.0, 0, PAGE_OFFSET_BITS) }

    pub const fn as_ptr<T>(self) -> *const T { self.0 as *const T }
    pub const fn as_mut_ptr<T>(self) -> *mut T { self.0 as *mut T }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::PAGE_SIZE;

    mod virtual_addr {
        use super::*;

        #[test]
        fn offset_extraction() {
            assert_eq!(VirtualAddr::new(0x8200_1ABC).offset(), 0xABC, "offset");
        }

        #[test]
        fn alignment() {
            assert!(VirtualAddr::new(0x8000_0000).is_aligned(PAGE_SIZE), "page-aligned");
            assert!(!VirtualAddr::new(0x8000_0001).is_aligned(PAGE_SIZE), "not page-aligned");
            assert!(VirtualAddr::new(0x8020_0000).is_aligned(2 * 1024 * 1024), "2 MiB aligned");
        }
    }

    mod physical_addr {
        use super::*;

        #[test]
        fn ppn_extraction() {
            let pa = PhysicalAddr::new(0x8200_1ABC);
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
