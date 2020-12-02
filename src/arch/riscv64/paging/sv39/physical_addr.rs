use crate::common::{extract_value, set_range};
use crate::{kprint, kprintln};
use alloc::format;
use bitflags::bitflags;
use core::fmt;
use core::ptr;

use super::PAGE_SIZE;

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

impl PhysicalAddr {
    pub const fn new(paddr: usize) -> Self { PhysicalAddr(paddr) }

    pub fn from(ppn: usize, offset: usize) -> Self {
        // let heading_bit_set = (1 << 43) & ppn == 0;

        // PhysicalAddr(((if heading_bit_set { 1 << 8 - 1 } else { 0 }) << 56) | (ppn << 12) | offset)

        let mut bits = set_range(0, ppn, 12, 56);
        bits = set_range(bits, offset, 0, 12);
        PhysicalAddr(bits)
    }

    pub const fn as_ptr<T>(&self) -> *const T { self.0 as *const T }
    pub const fn as_mut_ptr<T>(&self) -> *mut T { self.0 as *mut T }

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

    pub const fn is_aligned(&self, alignment: usize) -> bool {
        match self {
            PhysicalAddr(addr) => (*addr) % alignment == 0,
        }
    }
}
