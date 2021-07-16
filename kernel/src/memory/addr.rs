use core::fmt;
use core::mem::{size_of, size_of_val};

// | Not Used | PPN[2]  | PPN[1]  | PPN[0]  | Page Offset |
// +----------+---------+---------+---------+-------------+
// | 63 - 56  | 55 - 30 | 29 - 21 | 20 - 12 | 11 - 0      |
// +----------+---------+---------+---------+-------------+
#[repr(transparent)]
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct PhysicalAddr(u64);

impl fmt::Debug for PhysicalAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!(
            "PhysicalAddr({:#x} = {:#x},{:#x})",
            self.0,
            self.extract_ppn(),
            self.extract_offset()
        ))
    }
}

impl From<u64> for PhysicalAddr {
    fn from(paddr: u64) -> Self {
        PhysicalAddr(paddr)
    }
}

impl PhysicalAddr {
    pub fn new(ppn: u64, offset: u64) -> Self {
        let addr = store_range!(offset, 12, 56, ppn);
        PhysicalAddr(set_range!(addr, 56, 64, extract_nth_bit!(addr, 55)))
    }

    pub const fn extract_bits(&self) -> u64 {
        self.0
    }

    pub const fn as_ptr<T>(&self) -> *const T {
        self.0 as *const T
    }
    pub const fn as_mut_ptr<T>(&self) -> *mut T {
        self.0 as *mut T
    }

    pub fn extract_ppn(&self) -> u64 {
        extract_range!(self.0, 12, 56)
    }

    pub fn extract_offset(&self) -> u64 {
        extract_range!(self.0, 0, 12)
    }

    pub fn is_aligned(&self, alignment: u64) -> bool {
        self.extract_offset() == 0
    }
}

// +----------+---------+---------+---------+-------------+
// | Not Used | VPN[2]  | VPN[1]  | VPN[0]  | page offset |
// +----------+---------+---------+---------+-------------+
// | 63 - 39  | 38 - 30 | 29 - 21 | 20 - 12 | 11 - 0      |
// +----------+---------+---------+---------+-------------+

#[repr(transparent)]
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct VirtualAddr(u64);

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

impl From<u64> for VirtualAddr {
    fn from(vaddr: u64) -> Self {
        VirtualAddr(vaddr)
    }
}

impl VirtualAddr {
    pub fn new(vpn: u64, offset: u64) -> Self {
        let addr = store_range!(offset, 12, 56, vpn);
        VirtualAddr(set_range!(addr, 39, 64, extract_nth_bit!(addr, 38)))
    }

    pub const fn extract_bits(&self) -> u64 {
        self.0
    }

    pub const fn as_ptr<T>(&self) -> *const T {
        self.0 as *const T
    }
    pub const fn as_mut_ptr<T>(&self) -> *mut T {
        self.0 as *mut T
    }

    pub fn extract_vpn(&self, idx: u64) -> u64 {
        let mask = (1 << 9) - 1;
        match idx {
            0 => extract_range!(self.0, 12, 21),
            1 => extract_range!(self.0, 21, 30),
            2 => extract_range!(self.0, 30, 39),
            _ => panic!("[entry.extract_vpn] idx should be one of 0..=2"),
        }
    }

    pub fn extract_offset(&self) -> u64 {
        extract_range!(self.0, 0, 12)
    }

    pub fn is_aligned(&self, alignment: u64) -> bool {
        self.extract_offset() == 0
    }
}
