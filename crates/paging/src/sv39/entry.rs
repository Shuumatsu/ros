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

macro_rules! define_flag_methods {
    ($(($is:ident, $set:ident, $clear:ident, $flag:ident)),* $(,)?) => {
        $(
            pub const fn $is(&self) -> bool { (self.0 & $flag) != 0 }
            pub fn $set(&mut self) { self.0 |= $flag; }
            pub fn $clear(&mut self) { self.0 &= !$flag; }
        )*
    };
}

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

    define_flag_methods!(
        (is_valid, set_valid, clear_valid, VALID),
        (is_read, set_read, clear_read, READ),
        (is_write, set_write, clear_write, WRITE),
        (is_execute, set_execute, clear_execute, EXECUTE),
        (is_user, set_user, clear_user, USER),
        (is_global, set_global, clear_global, GLOBAL),
        (is_access, set_access, clear_access, ACCESS),
        (is_dirty, set_dirty, clear_dirty, DIRTY),
        (is_read_write, set_read_write, clear_read_write, READ_WRITE),
        (is_read_execute, set_read_execute, clear_read_execute, READ_EXECUTE),
        (
            is_read_write_execute,
            set_read_write_execute,
            clear_read_write_execute,
            READ_WRITE_EXECUTE
        ),
        (is_user_read_write, set_user_read_write, clear_user_read_write, USER_READ_WRITE),
        (is_user_read_execute, set_user_read_execute, clear_user_read_execute, USER_READ_EXECUTE),
        (
            is_user_read_write_execute,
            set_user_read_write_execute,
            clear_user_read_write_execute,
            USER_READ_WRITE_EXECUTE
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entry_new_and_bits() {
        let entry = Entry::new(0);
        assert!(!entry.is_valid());
        assert!(!entry.is_read());
        assert!(!entry.is_write());
        assert!(!entry.is_execute());

        let entry = Entry::new(0b11111111); // all flags set
        assert!(entry.is_valid());
        assert!(entry.is_read());
        assert!(entry.is_write());
        assert!(entry.is_execute());
        assert!(entry.is_user());
        assert!(entry.is_global());
        assert!(entry.is_access());
        assert!(entry.is_dirty());
    }

    #[test]
    fn test_entry_flag_constants() {
        // Verify flag bit positions match RISC-V spec
        assert_eq!(VALID, 1 << 0);
        assert_eq!(READ, 1 << 1);
        assert_eq!(WRITE, 1 << 2);
        assert_eq!(EXECUTE, 1 << 3);
        assert_eq!(USER, 1 << 4);
        assert_eq!(GLOBAL, 1 << 5);
        assert_eq!(ACCESS, 1 << 6);
        assert_eq!(DIRTY, 1 << 7);
    }

    #[test]
    fn test_entry_set_clear_flags() {
        let mut entry = Entry::new(0);

        // Test one flag to verify macro correctness
        entry.set_valid();
        assert!(entry.is_valid());
        entry.clear_valid();
        assert!(!entry.is_valid());
    }

    #[test]
    fn test_entry_leaf_vs_branch() {
        // Branch: V=1, R=W=X=0 (points to next level table)
        let branch = Entry::new(VALID);
        assert!(branch.is_branch());
        assert!(!branch.is_leaf());

        // Leaf: has at least one of R, W, X set
        let leaf_r = Entry::new(VALID | READ);
        assert!(leaf_r.is_leaf());
        assert!(!leaf_r.is_branch());

        let leaf_w = Entry::new(VALID | WRITE);
        assert!(leaf_w.is_leaf());

        let leaf_x = Entry::new(VALID | EXECUTE);
        assert!(leaf_x.is_leaf());

        let leaf_rwx = Entry::new(VALID | READ | WRITE | EXECUTE);
        assert!(leaf_rwx.is_leaf());
    }

    #[test]
    fn test_entry_ppn_extraction() {
        // PTE layout: bits [53:10] = PPN
        // PPN[0]: bits [18:10] (9 bits)
        // PPN[1]: bits [27:19] (9 bits)
        // PPN[2]: bits [53:28] (26 bits)

        // Set PPN[0] = 0x1FF (max 9-bit value)
        let entry = Entry::new(0x1FF << 10);
        assert_eq!(entry.extract_ppn(0), 0x1FF);
        assert_eq!(entry.extract_ppn(1), 0);
        assert_eq!(entry.extract_ppn(2), 0);

        // Set PPN[1] = 0x1FF
        let entry = Entry::new(0x1FF << 19);
        assert_eq!(entry.extract_ppn(0), 0);
        assert_eq!(entry.extract_ppn(1), 0x1FF);
        assert_eq!(entry.extract_ppn(2), 0);

        // Set PPN[2] = 0x3FFFFFF (max 26-bit value)
        let entry = Entry::new(0x3FFFFFF_usize << 28);
        assert_eq!(entry.extract_ppn(0), 0);
        assert_eq!(entry.extract_ppn(1), 0);
        assert_eq!(entry.extract_ppn(2), 0x3FFFFFF);
    }

    #[test]
    fn test_entry_ppn_all() {
        // Full 44-bit PPN
        let ppn_all: usize = 0xFFF_FFFFFFFF; // 44 bits all set
        let entry = Entry::new(ppn_all << 10);
        assert_eq!(entry.extract_ppn_all(), ppn_all);
    }

    #[test]
    fn test_entry_set_ppn() {
        let mut entry = Entry::new(VALID | READ);
        let paddr = PhysicalAddr::new(0x8020_0000); // typical kernel address

        entry.set_ppn(paddr);

        // Verify PPN was set correctly (paddr >> 12 = PPN)
        assert_eq!(entry.extract_ppn_all(), 0x8020_0000 >> 12);
        // Verify flags were preserved
        assert!(entry.is_valid());
        assert!(entry.is_read());
    }

    #[test]
    fn test_entry_set_flags_preserves_ppn() {
        let mut entry = Entry::new(0);
        let paddr = PhysicalAddr::new(0x8020_0000);

        entry.set_ppn(paddr);
        entry.set_flags(VALID | READ | WRITE);

        // PPN should still be intact
        assert_eq!(entry.extract_ppn_all(), 0x8020_0000 >> 12);
        assert!(entry.is_valid());
        assert!(entry.is_read());
        assert!(entry.is_write());
    }
}
