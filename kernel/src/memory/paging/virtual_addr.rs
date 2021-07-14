use crate::utils::{extract_value, set_range};
use bitflags::bitflags;
use core::fmt;
use core::ptr;

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

impl VirtualAddr {
    pub const fn new(vaddr: usize) -> Self {
        VirtualAddr(vaddr)
    }

    pub fn from(vpn: usize, offset: usize) -> Self {
        // let heading_bit_set = (1 << 26) & vpn == 0;

        // VirtualAddr(
        //     ((if heading_bit_set { (1 << 25) - 1 } else { 0 }) << 56) | (vpn << 12) | offset,
        // )

        let mut bits = set_range(0, vpn, 12, 39);
        bits = set_range(bits, offset, 0, 12);
        VirtualAddr(bits)
    }

    pub const fn as_ptr<T>(&self) -> *const T {
        self.0 as *const T
    }
    pub const fn as_mut_ptr<T>(&self) -> *mut T {
        self.0 as *mut T
    }

    pub fn extract_vpn(&self, idx: usize) -> usize {
        let mask = (1 << 9) - 1;
        match idx {
            0 => extract_value(self.0, mask, 12),
            1 => extract_value(self.0, mask, 21),
            2 => extract_value(self.0, mask, 30),
            _ => panic!("[entry.extract_vpn] idx should be one of 0..=2"),
        }
    }

    pub const fn extract_bits(&self) -> usize {
        self.0
    }

    pub fn extract_offset(&self) -> usize {
        extract_value(self.0, (1 << 12) - 1, 0)
    }
    pub fn set_offset(&mut self, offset: usize) -> Self {
        VirtualAddr(set_range(self.0, offset, 0, 12))
    }

    pub fn is_aligned(&self, alignment: usize) -> bool {
        match self {
            VirtualAddr(addr) => *addr % alignment == 0,
        }
    }
}
