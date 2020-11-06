use crate::utils::extract_value;
use bitflags::bitflags;
use core::ptr;
use lazy_static::lazy_static;
use spin::Mutex;

// +----------+---------+---------+---------+-------------+
// | Not Used | PPN[2]  | PPN[1]  | PPN[0]  | Page Offset |
// +----------+---------+---------+---------+-------------+
// | 63 - 56  | 55 - 30 | 29 - 21 | 20 - 12 | 11 - 0      |
// +----------+---------+---------+---------+-------------+
#[repr(transparent)]
pub struct PhysicalAddr(usize);
impl PhysicalAddr {
    pub fn extract_ppn(&self, idx: usize) -> Option<usize> {
        match idx {
            0 => Some(extract_value(self.0, (1 << 10) - 1, 12)),
            1 => Some(extract_value(self.0, (1 << 10) - 1, 21)),
            2 => Some(extract_value(self.0, (1 << 27) - 1, 30)),
            _ => None,
        }
    }

    pub fn extract_ppn_all(&self) -> usize {
        extract_value(self.0, (1 << 45) - 1, 12)
    }

    pub fn extract_offset(&self) -> usize {
        extract_value(self.0, (1 << 13) - 1, 0)
    }
}
