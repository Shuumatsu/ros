//! Virtual and physical address types.
//!
//! A physical address is 56 bits and a page number 44 in every RV64 translation scheme, so
//! the physical decomposition lives here. A virtual address is read differently under each:
//! how many VPN fields it has and where its sign extension begins both follow from a
//! scheme's level count, so [`Scheme`](crate::Scheme) owns that half.
//!
//! The types are also the crate's contribution to code that never walks a page table:
//! `PhysicalAddr` is what keeps a physical address from being passed where a pointer is
//! wanted, which in a higher-half kernel is the difference between a fault and a wrong
//! answer.

use crate::geometry::{PAGE_OFFSET_BITS, PPN_BITS};
use crate::utils::{align_down, align_offset, align_up, field, with_field};
use core::fmt;

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

/// A virtual address.
#[repr(transparent)]
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct VirtualAddr(usize);

/// The bare address: how many VPN fields it decomposes into is a scheme's answer, and this
/// type has no scheme to ask.
impl fmt::Debug for VirtualAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "VirtualAddr({:#x})", self.0)
    }
}

/// Formats as the bare address, so `{:#x}` works and width flags are honoured.
///
/// The alternative is `.bits()` — and a caller that reaches for it to print has just
/// dropped the type for the rest of the expression.
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

/// A 56-bit physical address.
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

    /// Build an address from a physical page number and a byte offset.
    pub fn from_parts(ppn: usize, offset: usize) -> Self {
        let bits = with_field(0, PAGE_OFFSET_BITS, PPN_BITS, ppn);
        Self(with_field(bits, 0, PAGE_OFFSET_BITS, offset))
    }

    /// The page-aligned physical address of the frame numbered `ppn`.
    pub const fn from_ppn(ppn: usize) -> Self { Self(ppn << PAGE_OFFSET_BITS) }

    pub const fn bits(self) -> usize { self.0 }

    /// The full 44-bit physical page number (`addr >> 12`).
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
