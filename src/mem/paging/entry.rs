use crate::utils::extract_value;
use bitflags::bitflags;
use core::ptr;
use lazy_static::lazy_static;
use spin::Mutex;

// +----------+---------+---------+---------+-------+---+---+---+---+---+---+---+---+
// | Not Used | PPN[2]  | PPN[1]  | PPN[0]  | RSW   | D | A | G | U | X | W | R | V |
// +----------+---------+---------+---------+-------+---+---+---+---+---+---+---+---+
// | 63 - 54  | 53 - 28 | 27 - 19 | 18 - 10 | 9 - 8 | 7 | 6 | 5 | 4 | 3 | 2 | 1 | 0 |
// +----------+---------+---------+---------+-------+---+---+---+---+---+---+---+---+
#[repr(transparent)]
pub struct Entry(usize);
// bitflags! {
//     pub struct EntryFlags: u64 {
//         const Valid     = 1 << 0;
//         const Read      = 1 << 1;
//         const Write     = 1 << 2;
//         const Execute   = 1 << 3;
//         const User      = 1 << 4;
//         const Global    = 1 << 5;
//         const Access    = 1 << 6;
//         const Dirty     = 1 << 7;

//         const ReadWrite = Self::Read.bits | Self::Write.bits;
//         const ReadExecute = Self::Read.bits | Self::Execute.bits;
//         const ReadWriteExecute = Self::Read.bits | Self::Write.bits | Self::Execute.bits;

//         const UserReadWrite = Self::ReadWrite.bits | Self::User.bits;
//         const UserReadExecute = Self::ReadExecute.bits | Self::User.bits;
//         const UserReadWriteExecute = Self::UserReadWriteExecute.bits | Self::User.bits;
//   }
// }
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
pub const USER_READ_WRITE_EXECUTE: usize = USER_READ_WRITE_EXECUTE | USER;

impl Entry {
    pub fn new(bits: usize) -> Self {
        Entry(bits)
    }

    pub fn extract_ppn(&self, idx: usize) -> Option<usize> {
        match idx {
            0 => Some(extract_value(self.0, (1 << 10) - 1, 10)),
            1 => Some(extract_value(self.0, (1 << 10) - 1, 19)),
            2 => Some(extract_value(self.0, (1 << 27) - 1, 28)),
            _ => None,
        }
    }

    pub fn extract_ppn_all(&self) -> usize {
        extract_value(self.0, (1 << 45) - 1, 10)
    }

    pub fn is_valid(&self) -> bool {
        self.0 & VALID != 0
    }
    pub fn set_valid(&mut self) {
        self.0 &= VALID
    }
    pub fn clear_valid(&mut self) {
        self.0 &= !VALID
    }

    pub fn is_read(&self) -> bool {
        self.0 & READ != 0
    }
    pub fn set_read(&mut self) {
        self.0 &= READ
    }
    pub fn clear_read(&mut self) {
        self.0 &= !READ
    }

    pub fn is_write(&self) -> bool {
        self.0 & WRITE != 0
    }
    pub fn set_write(&mut self) {
        self.0 &= WRITE
    }
    pub fn clear_write(&mut self) {
        self.0 &= !WRITE
    }

    pub fn is_execute(&self) -> bool {
        self.0 & EXECUTE != 0
    }
    pub fn set_execute(&mut self) {
        self.0 &= EXECUTE
    }
    pub fn clear_execute(&mut self) {
        self.0 &= !EXECUTE
    }

    pub fn is_user(&self) -> bool {
        self.0 & USER != 0
    }
    pub fn set_user(&mut self) {
        self.0 &= USER
    }
    pub fn clear_user(&mut self) {
        self.0 &= !USER
    }

    pub fn is_global(&self) -> bool {
        self.0 & GLOBAL != 0
    }
    pub fn set_global(&mut self) {
        self.0 &= GLOBAL
    }
    pub fn clear_global(&mut self) {
        self.0 &= !GLOBAL
    }

    pub fn is_access(&self) -> bool {
        self.0 & ACCESS != 0
    }
    pub fn set_access(&mut self) {
        self.0 &= ACCESS
    }
    pub fn clear_access(&mut self) {
        self.0 &= !ACCESS
    }

    pub fn is_dirty(&self) -> bool {
        self.0 & DIRTY != 0
    }
    pub fn set_dirty(&mut self) {
        self.0 &= DIRTY
    }
    pub fn clear_dirty(&mut self) {
        self.0 &= !DIRTY
    }

    pub fn is_read_write(&self) -> bool {
        self.0 & READ_WRITE != 0
    }
    pub fn set_read_write(&mut self) {
        self.0 &= READ_WRITE
    }
    pub fn clear_read_write(&mut self) {
        self.0 &= !READ_WRITE
    }

    pub fn is_read_execute(&self) -> bool {
        self.0 & READ_EXECUTE != 0
    }
    pub fn set_read_execute(&mut self) {
        self.0 &= READ_EXECUTE
    }
    pub fn clear_read_execute(&mut self) {
        self.0 &= !READ_EXECUTE
    }

    pub fn is_read_write_execute(&self) -> bool {
        self.0 & READ_WRITE_EXECUTE != 0
    }
    pub fn set_read_write_execute(&mut self) {
        self.0 &= READ_WRITE_EXECUTE
    }
    pub fn clear_read_write_execute(&mut self) {
        self.0 &= !READ_WRITE_EXECUTE
    }

    pub fn is_user_read_write(&self) -> bool {
        self.0 & USER_READ_WRITE != 0
    }
    pub fn set_user_read_write(&mut self) {
        self.0 &= USER_READ_WRITE
    }
    pub fn clear_user_read_write(&mut self) {
        self.0 &= !USER_READ_WRITE
    }

    pub fn is_user_read_execute(&self) -> bool {
        self.0 & USER_READ_EXECUTE != 0
    }
    pub fn set_user_read_execute(&mut self) {
        self.0 &= USER_READ_EXECUTE
    }
    pub fn clear_user_read_execute(&mut self) {
        self.0 &= !USER_READ_EXECUTE
    }

    pub fn is_user_read_write_execute(&self) -> bool {
        self.0 & USER_READ_WRITE_EXECUTE != 0
    }
    pub fn set_user_read_write_execute(&mut self) {
        self.0 &= USER_READ_WRITE_EXECUTE
    }
    pub fn clear_user_read_write_execute(&mut self) {
        self.0 &= !USER_READ_WRITE_EXECUTE
    }
}
