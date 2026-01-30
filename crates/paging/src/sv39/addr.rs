//! Virtual and physical address types for Sv39 paging.

use crate::utils::{align_down, align_offset, align_up, extract_value, set_range};
use core::fmt;

/// Trait for memory address types providing common operations.
pub trait MemoryAddr: Copy + Clone + Ord + Eq {
    fn from_usize(addr: usize) -> Self;
    fn as_usize(self) -> usize;

    // Alignment
    fn align_down(self, align: usize) -> Self {
        Self::from_usize(align_down(self.as_usize(), align))
    }
    fn align_up(self, align: usize) -> Self {
        Self::from_usize(align_up(self.as_usize(), align))
    }
    fn align_offset(self, align: usize) -> usize {
        align_offset(self.as_usize(), align)
    }
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
        (self.as_usize() as isize)
            .checked_sub(base.as_usize() as isize)
            .expect("offset overflow")
    }

    // Addition
    fn add(self, rhs: usize) -> Self {
        Self::from_usize(self.as_usize().checked_add(rhs).expect("address overflow"))
    }
    fn wrapping_add(self, rhs: usize) -> Self {
        Self::from_usize(self.as_usize().wrapping_add(rhs))
    }
    fn overflowing_add(self, rhs: usize) -> (Self, bool) {
        let (val, overflow) = self.as_usize().overflowing_add(rhs);
        (Self::from_usize(val), overflow)
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
    fn overflowing_sub(self, rhs: usize) -> (Self, bool) {
        let (val, overflow) = self.as_usize().overflowing_sub(rhs);
        (Self::from_usize(val), overflow)
    }
    fn checked_sub(self, rhs: usize) -> Option<Self> {
        self.as_usize().checked_sub(rhs).map(Self::from_usize)
    }

    // Address-to-address subtraction
    fn sub_addr(self, rhs: Self) -> usize {
        self.as_usize()
            .checked_sub(rhs.as_usize())
            .expect("address underflow")
    }
    fn wrapping_sub_addr(self, rhs: Self) -> usize {
        self.as_usize().wrapping_sub(rhs.as_usize())
    }
    fn overflowing_sub_addr(self, rhs: Self) -> (usize, bool) {
        self.as_usize().overflowing_sub(rhs.as_usize())
    }
    fn checked_sub_addr(self, rhs: Self) -> Option<usize> {
        self.as_usize().checked_sub(rhs.as_usize())
    }
}

// +----------+---------+---------+---------+-------------+
// | Not Used | VPN[2]  | VPN[1]  | VPN[0]  | page offset |
// +----------+---------+---------+---------+-------------+
// | 63 - 39  | 38 - 30 | 29 - 21 | 20 - 12 | 11 - 0      |
// +----------+---------+---------+---------+-------------+

#[repr(transparent)]
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct VirtualAddr(usize);

impl fmt::Debug for VirtualAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!(
            "VirtualAddr({:#x}: vpn[2]: {}, vpn[1]: {}, vpn[0]: {}, offset: {:#x})",
            self.0,
            self.extract_vpn(2),
            self.extract_vpn(1),
            self.extract_vpn(0),
            self.extract_offset()
        ))
    }
}

impl MemoryAddr for VirtualAddr {
    fn from_usize(addr: usize) -> Self { Self(addr) }
    fn as_usize(self) -> usize { self.0 }
}

impl VirtualAddr {
    pub const fn new(vaddr: usize) -> Self { VirtualAddr(vaddr) }

    pub fn from(vpn: usize, offset: usize) -> Self {
        let mut bits = set_range(0, vpn, 12, 39);
        bits = set_range(bits, offset, 0, 12);
        VirtualAddr(bits)
    }

    pub const fn as_ptr<T>(&self) -> *const T { self.0 as *const T }
    pub const fn as_mut_ptr<T>(&self) -> *mut T { self.0 as *mut T }

    pub fn extract_vpn(&self, idx: usize) -> usize {
        let mask = (1 << 9) - 1;
        match idx {
            0 => extract_value(self.0, mask, 12),
            1 => extract_value(self.0, mask, 21),
            2 => extract_value(self.0, mask, 30),
            _ => panic!("[entry.extract_vpn] idx should be one of 0..=2"),
        }
    }

    pub const fn extract_bits(&self) -> usize { self.0 }

    pub fn extract_offset(&self) -> usize { extract_value(self.0, (1 << 12) - 1, 0) }
    pub fn set_offset(&mut self, offset: usize) -> Self {
        VirtualAddr(set_range(self.0, offset, 0, 12))
    }

    // Pointer constructors (VirtualAddr only)
    pub fn from_ptr_of<T>(ptr: *const T) -> Self { Self::new(ptr as usize) }
    pub fn from_mut_ptr_of<T>(ptr: *mut T) -> Self { Self::new(ptr as usize) }
    pub const fn as_ptr_u8(self) -> *const u8 { self.0 as *const u8 }
    pub const fn as_mut_ptr_u8(self) -> *mut u8 { self.0 as *mut u8 }
}

// +----------+---------+---------+---------+-------------+
// | Not Used | PPN[2]  | PPN[1]  | PPN[0]  | Page Offset |
// +----------+---------+---------+---------+-------------+
// | 63 - 56  | 55 - 30 | 29 - 21 | 20 - 12 | 11 - 0      |
// +----------+---------+---------+---------+-------------+
#[repr(transparent)]
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct PhysicalAddr(usize);

impl fmt::Debug for PhysicalAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!(
            "PhysicalAddr({:#x}, ppn: {:#x}, offset: {:#x})",
            self.0,
            self.extract_ppn_all(),
            self.extract_offset()
        ))
    }
}

impl MemoryAddr for PhysicalAddr {
    fn from_usize(addr: usize) -> Self { Self(addr) }
    fn as_usize(self) -> usize { self.0 }
}

impl PhysicalAddr {
    pub const fn new(paddr: usize) -> Self { PhysicalAddr(paddr) }

    pub fn from(ppn: usize, offset: usize) -> Self {
        let mut bits = set_range(0, ppn, 12, 56);
        bits = set_range(bits, offset, 0, 12);
        PhysicalAddr(bits)
    }

    pub const fn as_ptr<T>(&self) -> *const T { self.0 as *const T }
    pub const fn as_mut_ptr<T>(&self) -> *mut T { self.0 as *mut T }

    pub const fn extract_bits(&self) -> usize { self.0 }

    pub const fn extract_ppn(&self, idx: usize) -> usize {
        match idx {
            0 => extract_value(self.0, (1 << 9) - 1, 12),
            1 => extract_value(self.0, (1 << 9) - 1, 21),
            2 => extract_value(self.0, (1 << 26) - 1, 30),
            _ => panic!("[paddr.extract_ppn] idx should be one of 0..=2"),
        }
    }

    pub const fn extract_ppn_all(&self) -> usize { extract_value(self.0, (1 << 44) - 1, 12) }

    pub const fn extract_offset(&self) -> usize { extract_value(self.0, (1 << 12) - 1, 0) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sv39::PAGE_SIZE;

    mod virtual_addr_tests {
        use super::*;

        #[test]
        fn test_vaddr_new() {
            let vaddr = VirtualAddr::new(0x8000_0000);
            assert_eq!(vaddr.extract_bits(), 0x8000_0000);
        }

        #[test]
        fn test_vaddr_vpn_extraction() {
            // VirtualAddr layout for Sv39:
            // VPN[0]: bits [20:12] (9 bits) - index into level 0 table
            // VPN[1]: bits [29:21] (9 bits) - index into level 1 table
            // VPN[2]: bits [38:30] (9 bits) - index into level 2 table
            // Offset: bits [11:0] (12 bits)

            // Address: 0x8200_1ABC
            // Binary breakdown:
            //   VPN[2] = (0x8200_1ABC >> 30) & 0x1FF = 2
            //   VPN[1] = (0x8200_1ABC >> 21) & 0x1FF = 16 (0x10)
            //   VPN[0] = (0x8200_1ABC >> 12) & 0x1FF = 1
            //   Offset = 0x8200_1ABC & 0xFFF = 0xABC

            let vaddr = VirtualAddr::new(0x8200_1ABC);
            assert_eq!(vaddr.extract_vpn(2), 2);
            assert_eq!(vaddr.extract_vpn(1), 16);
            assert_eq!(vaddr.extract_vpn(0), 1);
            assert_eq!(vaddr.extract_offset(), 0xABC);
        }

        #[test]
        fn test_vaddr_from_vpn_offset() {
            // Construct address from VPN and offset
            let vpn = (2 << 18) | (16 << 9) | 1; // VPN[2]=2, VPN[1]=16, VPN[0]=1
            let offset = 0xABC;
            let vaddr = VirtualAddr::from(vpn, offset);

            assert_eq!(vaddr.extract_vpn(2), 2);
            assert_eq!(vaddr.extract_vpn(1), 16);
            assert_eq!(vaddr.extract_vpn(0), 1);
            assert_eq!(vaddr.extract_offset(), 0xABC);
        }

        #[test]
        fn test_vaddr_max_vpn_values() {
            // Each VPN field is 9 bits, max value = 511 (0x1FF)
            let max_vpn = (0x1FF << 18) | (0x1FF << 9) | 0x1FF;
            let vaddr = VirtualAddr::from(max_vpn, 0xFFF);

            assert_eq!(vaddr.extract_vpn(2), 0x1FF);
            assert_eq!(vaddr.extract_vpn(1), 0x1FF);
            assert_eq!(vaddr.extract_vpn(0), 0x1FF);
            assert_eq!(vaddr.extract_offset(), 0xFFF);
        }

        #[test]
        fn test_vaddr_alignment() {
            let page_aligned = VirtualAddr::new(0x8000_0000);
            assert!(page_aligned.is_aligned(PAGE_SIZE));

            let not_aligned = VirtualAddr::new(0x8000_0001);
            assert!(!not_aligned.is_aligned(PAGE_SIZE));

            // 2MB (megapage) alignment
            let mega_aligned = VirtualAddr::new(0x8020_0000);
            assert!(mega_aligned.is_aligned(2 * 1024 * 1024));
        }

        #[test]
        fn test_vaddr_zero() {
            let vaddr = VirtualAddr::new(0);
            assert_eq!(vaddr.extract_vpn(0), 0);
            assert_eq!(vaddr.extract_vpn(1), 0);
            assert_eq!(vaddr.extract_vpn(2), 0);
            assert_eq!(vaddr.extract_offset(), 0);
        }
    }

    mod physical_addr_tests {
        use super::*;

        #[test]
        fn test_paddr_new() {
            let paddr = PhysicalAddr::new(0x8000_0000);
            assert_eq!(paddr.extract_bits(), 0x8000_0000);
        }

        #[test]
        fn test_paddr_ppn_extraction() {
            // PhysicalAddr layout for Sv39:
            // PPN[0]: bits [20:12] (9 bits)
            // PPN[1]: bits [29:21] (9 bits)
            // PPN[2]: bits [55:30] (26 bits)
            // Offset: bits [11:0] (12 bits)

            let paddr = PhysicalAddr::new(0x8200_1ABC);
            assert_eq!(paddr.extract_ppn(2), 2);
            assert_eq!(paddr.extract_ppn(1), 16);
            assert_eq!(paddr.extract_ppn(0), 1);
            assert_eq!(paddr.extract_offset(), 0xABC);
        }

        #[test]
        fn test_paddr_ppn_all() {
            let paddr = PhysicalAddr::new(0x8020_1ABC);
            // PPN_all = address >> 12
            assert_eq!(paddr.extract_ppn_all(), 0x8020_1ABC >> 12);
        }

        #[test]
        fn test_paddr_from_ppn_offset() {
            let ppn = 0x80201; // full PPN
            let offset = 0xABC;
            let paddr = PhysicalAddr::from(ppn, offset);

            assert_eq!(paddr.extract_ppn_all(), ppn);
            assert_eq!(paddr.extract_offset(), offset);
            assert_eq!(paddr.extract_bits(), (ppn << 12) | offset);
        }

        #[test]
        fn test_paddr_alignment() {
            let page_aligned = PhysicalAddr::new(0x8000_0000);
            assert!(page_aligned.is_aligned(PAGE_SIZE));

            let not_aligned = PhysicalAddr::new(0x8000_0001);
            assert!(!not_aligned.is_aligned(PAGE_SIZE));
        }

        #[test]
        fn test_paddr_roundtrip() {
            // Verify that from(ppn, offset) and extract work together
            for ppn in [0, 1, 0x80201, 0xFFF_FFFFFFFF_u64 as usize] {
                for offset in [0, 1, 0xFFF] {
                    // Only test valid 44-bit PPNs
                    let ppn = ppn & ((1 << 44) - 1);
                    let paddr = PhysicalAddr::from(ppn, offset);
                    assert_eq!(paddr.extract_ppn_all(), ppn, "PPN mismatch for ppn={ppn:#x}");
                    assert_eq!(
                        paddr.extract_offset(),
                        offset,
                        "Offset mismatch for offset={offset:#x}"
                    );
                }
            }
        }
    }
}
