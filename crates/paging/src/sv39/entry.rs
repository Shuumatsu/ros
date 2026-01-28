//! Page table entry type and flag constants for Sv39 paging.

use crate::utils::{extract_value, set_range};
use core::fmt;
use core::mem::size_of;

use super::ENTRY_SIZE;
use super::addr::PhysicalAddr;

// +----------+---------+---------+---------+-------+---+---+---+---+---+---+---+---+
// | Not Used | PPN[2]  | PPN[1]  | PPN[0]  | RSW   | D | A | G | U | X | W | R | V |
// +----------+---------+---------+---------+-------+---+---+---+---+---+---+---+---+
// | 63 - 54  | 53 - 28 | 27 - 19 | 18 - 10 | 9 - 8 | 7 | 6 | 5 | 4 | 3 | 2 | 1 | 0 |
// +----------+---------+---------+---------+-------+---+---+---+---+---+---+---+---+

pub const VALID: usize = 1 << 0;
pub const READ: usize = 1 << 1;
pub const WRITE: usize = 1 << 2;
pub const EXECUTE: usize = 1 << 3;
pub const USER: usize = 1 << 4;
pub const GLOBAL: usize = 1 << 5;
pub const ACCESS: usize = 1 << 6;
pub const DIRTY: usize = 1 << 7;

pub const READ_WRITE: usize = READ | WRITE;
pub const READ_EXECUTE: usize = READ | EXECUTE;
pub const READ_WRITE_EXECUTE: usize = READ | WRITE | EXECUTE;

pub const USER_READ_WRITE: usize = READ_WRITE | USER;
pub const USER_READ_EXECUTE: usize = READ_EXECUTE | USER;
pub const USER_READ_WRITE_EXECUTE: usize = READ_WRITE_EXECUTE | USER;

#[derive(Copy, Clone)]
#[repr(transparent)]
pub struct Entry(usize);
const_assert_eq!(size_of::<Entry>(), ENTRY_SIZE);

impl fmt::Debug for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!(
            "Entry({:#x}, ppn[2]: {}, ppn[1]: {}, ppn[0]: {}, flags: {:#010b})",
            self.0,
            self.extract_ppn(2),
            self.extract_ppn(1),
            self.extract_ppn(0),
            extract_value(self.0, (1 << 8) - 1, 0)
        ))
    }
}

unsafe impl Send for Entry {}

impl Entry {
    pub const fn new(bits: usize) -> Self { Entry(bits) }

    pub fn set_bits(&mut self, bits: usize) { self.0 = bits }

    pub fn set_flags(&mut self, flags: usize) { self.0 = set_range(self.0, flags, 0, 8); }

    pub const fn extract_ppn(&self, idx: usize) -> usize {
        match idx {
            0 => extract_value(self.0, (1 << 9) - 1, 10),
            1 => extract_value(self.0, (1 << 9) - 1, 19),
            2 => extract_value(self.0, (1 << 26) - 1, 28),
            _ => panic!("[entry.extract_ppn] idx should be one of 0..=2"),
        }
    }

    pub const fn extract_ppn_all(&self) -> usize { extract_value(self.0, (1 << 44) - 1, 10) }

    pub fn set_ppn(&mut self, paddr: PhysicalAddr) {
        self.0 = set_range(self.0, paddr.extract_ppn_all(), 10, 54)
    }

    // A leaf has one or more RWX bits set
    pub const fn is_leaf(&self) -> bool { (self.0 & (READ | WRITE | EXECUTE)) != 0 }
    pub const fn is_branch(&self) -> bool { !self.is_leaf() }

    pub const fn is_valid(&self) -> bool { (self.0 & VALID) != 0 }
    pub fn set_valid(&mut self) { self.0 |= VALID }
    pub fn clear_valid(&mut self) { self.0 &= !VALID }

    pub const fn is_read(&self) -> bool { (self.0 & READ) != 0 }
    pub fn set_read(&mut self) { self.0 |= READ }
    pub fn clear_read(&mut self) { self.0 &= !READ }

    pub const fn is_write(&self) -> bool { (self.0 & WRITE) != 0 }
    pub fn set_write(&mut self) { self.0 |= WRITE }
    pub fn clear_write(&mut self) { self.0 &= !WRITE }

    pub const fn is_execute(&self) -> bool { (self.0 & EXECUTE) != 0 }
    pub fn set_execute(&mut self) { self.0 |= EXECUTE }
    pub fn clear_execute(&mut self) { self.0 &= !EXECUTE }

    pub const fn is_user(&self) -> bool { (self.0 & USER) != 0 }
    pub fn set_user(&mut self) { self.0 |= USER }
    pub fn clear_user(&mut self) { self.0 &= !USER }

    pub const fn is_global(&self) -> bool { (self.0 & GLOBAL) != 0 }
    pub fn set_global(&mut self) { self.0 |= GLOBAL }
    pub fn clear_global(&mut self) { self.0 &= !GLOBAL }

    pub const fn is_access(&self) -> bool { (self.0 & ACCESS) != 0 }
    pub fn set_access(&mut self) { self.0 |= ACCESS }
    pub fn clear_access(&mut self) { self.0 &= !ACCESS }

    pub const fn is_dirty(&self) -> bool { (self.0 & DIRTY) != 0 }
    pub fn set_dirty(&mut self) { self.0 |= DIRTY }
    pub fn clear_dirty(&mut self) { self.0 &= !DIRTY }

    pub const fn is_read_write(&self) -> bool { (self.0 & READ_WRITE) != 0 }
    pub fn set_read_write(&mut self) { self.0 |= READ_WRITE }
    pub fn clear_read_write(&mut self) { self.0 &= !READ_WRITE }

    pub const fn is_read_execute(&self) -> bool { (self.0 & READ_EXECUTE) != 0 }
    pub fn set_read_execute(&mut self) { self.0 |= READ_EXECUTE }
    pub fn clear_read_execute(&mut self) { self.0 &= !READ_EXECUTE }

    pub const fn is_read_write_execute(&self) -> bool { (self.0 & READ_WRITE_EXECUTE) != 0 }
    pub fn set_read_write_execute(&mut self) { self.0 |= READ_WRITE_EXECUTE }
    pub fn clear_read_write_execute(&mut self) { self.0 &= !READ_WRITE_EXECUTE }

    pub const fn is_user_read_write(&self) -> bool { (self.0 & USER_READ_WRITE) != 0 }
    pub fn set_user_read_write(&mut self) { self.0 |= USER_READ_WRITE }
    pub fn clear_user_read_write(&mut self) { self.0 &= !USER_READ_WRITE }

    pub const fn is_user_read_execute(&self) -> bool { (self.0 & USER_READ_EXECUTE) != 0 }
    pub fn set_user_read_execute(&mut self) { self.0 |= USER_READ_EXECUTE }
    pub fn clear_user_read_execute(&mut self) { self.0 &= !USER_READ_EXECUTE }

    pub const fn is_user_read_write_execute(&self) -> bool {
        (self.0 & USER_READ_WRITE_EXECUTE) != 0
    }
    pub fn set_user_read_write_execute(&mut self) { self.0 |= USER_READ_WRITE_EXECUTE }
    pub fn clear_user_read_write_execute(&mut self) { self.0 &= !USER_READ_WRITE_EXECUTE }
}
