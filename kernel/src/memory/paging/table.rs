use alloc::alloc::{alloc_zeroed, dealloc, Layout};
use core::fmt;
use core::mem::size_of;
use core::ptr;

use crate::config::{ENTRIES_PER_PAGE, ENTRY_SIZE, PAGE_SIZE, PAGE_SIZE_BITS};

pub use super::physical_addr::*;
pub use super::virtual_addr::*;

// +----------+---------+---------+---------+-------+---+---+---+---+---+---+---+---+
// | Not Used | PPN[2]  | PPN[1]  | PPN[0]  | RSW   | D | A | G | U | X | W | R | V |
// +----------+---------+---------+---------+-------+---+---+---+---+---+---+---+---+
// | 63 - 54  | 53 - 28 | 27 - 19 | 18 - 10 | 9 - 8 | 7 | 6 | 5 | 4 | 3 | 2 | 1 | 0 |
// +----------+---------+---------+---------+-------+---+---+---+---+---+---+---+---+
bitflags! {
    pub struct EntryFlags: u8 {
        const Valid     = 1 << 0;
        const Read      = 1 << 1;
        const Write     = 1 << 2;
        const Execute   = 1 << 3;
        const User      = 1 << 4;
        const Global    = 1 << 5;
        const Access    = 1 << 6;
        const Dirty     = 1 << 7;

        const ReadWrite = Self::Read.bits | Self::Write.bits;
        const ReadExecute = Self::Read.bits | Self::Execute.bits;
        const ReadWriteExecute = Self::Read.bits | Self::Write.bits | Self::Execute.bits;

        const UserReadWrite = Self::ReadWrite.bits | Self::User.bits;
        const UserReadExecute = Self::ReadExecute.bits | Self::User.bits;
        const UserReadWriteExecute = Self::UserReadWriteExecute.bits | Self::User.bits;
  }
}

#[derive(Copy, Clone)]
#[repr(transparent)]
pub struct Entry(u64);
const_assert_eq!(size_of::<Entry>(), ENTRY_SIZE);

unsafe impl Send for Entry {}

impl Entry {
    pub const fn new(bits: u64) -> Self {
        Entry(bits)
    }

    pub fn set_bits(&mut self, bits: u64) {
        self.0 = bits
    }

    pub fn get_flags(&self) -> EntryFlags {}
    pub fn set_flags(&mut self, flags: EntryFlags) {
        self.0 = set_range!(self.0, flags.bits(), 0, 8);
    }

    pub const fn extract_ppn(&self, idx: u64) -> u64 {
        match idx {
            0 => extract_value!(self.0, (1 << 9) - 1, 10),
            1 => extract_value!(self.0, (1 << 9) - 1, 19),
            2 => extract_value!(self.0, (1 << 26) - 1, 28),
            _ => panic!("[entry.extract_ppn] idx should be one of 0..=2"),
        }
    }

    pub const fn extract_ppn_all(&self) -> u64 {
        extract_value!(self.0, (1 << 44) - 1, 10)
    }

    pub fn set_ppn(&mut self, paddr: PhysicalAddr) {
        self.0 = set_range!(self.0, paddr.extract_ppn_all(), 10, 54)
    }

    // A leaf has one or more RWX bits set
    pub const fn is_leaf(&self) -> bool {
        let flags = EntryFlags::from_bits_truncate(self.0);
        flags.intersects(EntryFlags::Read | EntryFlags::Write | EntryFlags::Execute)
    }
    pub const fn is_branch(&self) -> bool {
        !self.is_leaf()
    }

    pub const fn is_valid(&self) -> bool {
        let flags = EntryFlags::from_bits_truncate(self.0);
        flags.intersects(EntryFlags::Valid)
    }
    pub fn set_valid(&mut self) {
        self.0 |= VALID
    }
    pub fn clear_valid(&mut self) {
        self.0 &= !VALID
    }

    pub const fn is_read(&self) -> bool {
        (self.0 & READ) != 0
    }
    pub fn set_read(&mut self) {
        self.0 |= READ
    }
    pub fn clear_read(&mut self) {
        self.0 &= !READ
    }

    pub const fn is_write(&self) -> bool {
        (self.0 & WRITE) != 0
    }
    pub fn set_write(&mut self) {
        self.0 |= WRITE
    }
    pub fn clear_write(&mut self) {
        self.0 &= !WRITE
    }

    pub const fn is_execute(&self) -> bool {
        (self.0 & EXECUTE) != 0
    }
    pub fn set_execute(&mut self) {
        self.0 |= EXECUTE
    }
    pub fn clear_execute(&mut self) {
        self.0 &= !EXECUTE
    }

    pub const fn is_user(&self) -> bool {
        (self.0 & USER) != 0
    }
    pub fn set_user(&mut self) {
        self.0 |= USER
    }
    pub fn clear_user(&mut self) {
        self.0 &= !USER
    }

    pub const fn is_global(&self) -> bool {
        (self.0 & GLOBAL) != 0
    }
    pub fn set_global(&mut self) {
        self.0 |= GLOBAL
    }
    pub fn clear_global(&mut self) {
        self.0 &= !GLOBAL
    }

    pub const fn is_access(&self) -> bool {
        (self.0 & ACCESS) != 0
    }
    pub fn set_access(&mut self) {
        self.0 |= ACCESS
    }
    pub fn clear_access(&mut self) {
        self.0 &= !ACCESS
    }

    pub const fn is_dirty(&self) -> bool {
        (self.0 & DIRTY) != 0
    }
    pub fn set_dirty(&mut self) {
        self.0 |= DIRTY
    }
    pub fn clear_dirty(&mut self) {
        self.0 &= !DIRTY
    }

    pub const fn is_read_write(&self) -> bool {
        (self.0 & READ_WRITE) != 0
    }
    pub fn set_read_write(&mut self) {
        self.0 |= READ_WRITE
    }
    pub fn clear_read_write(&mut self) {
        self.0 &= !READ_WRITE
    }

    pub const fn is_read_execute(&self) -> bool {
        (self.0 & READ_EXECUTE) != 0
    }
    pub fn set_read_execute(&mut self) {
        self.0 |= READ_EXECUTE
    }
    pub fn clear_read_execute(&mut self) {
        self.0 &= !READ_EXECUTE
    }

    pub const fn is_read_write_execute(&self) -> bool {
        (self.0 & READ_WRITE_EXECUTE) != 0
    }
    pub fn set_read_write_execute(&mut self) {
        self.0 |= READ_WRITE_EXECUTE
    }
    pub fn clear_read_write_execute(&mut self) {
        self.0 &= !READ_WRITE_EXECUTE
    }

    pub const fn is_user_read_write(&self) -> bool {
        (self.0 & USER_READ_WRITE) != 0
    }
    pub fn set_user_read_write(&mut self) {
        self.0 |= USER_READ_WRITE
    }
    pub fn clear_user_read_write(&mut self) {
        self.0 &= !USER_READ_WRITE
    }

    pub const fn is_user_read_execute(&self) -> bool {
        (self.0 & USER_READ_EXECUTE) != 0
    }
    pub fn set_user_read_execute(&mut self) {
        self.0 |= USER_READ_EXECUTE
    }
    pub fn clear_user_read_execute(&mut self) {
        self.0 &= !USER_READ_EXECUTE
    }

    pub const fn is_user_read_write_execute(&self) -> bool {
        (self.0 & USER_READ_WRITE_EXECUTE) != 0
    }
    pub fn set_user_read_write_execute(&mut self) {
        self.0 |= USER_READ_WRITE_EXECUTE
    }
    pub fn clear_user_read_write_execute(&mut self) {
        self.0 &= !USER_READ_WRITE_EXECUTE
    }
}

impl fmt::Debug for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!(
            "Entry({:#x}, ppn[2]: {}, ppn[1]: {}, ppn[0]: {}, flags: {:?})",
            self.0,
            self.extract_ppn(2),
            self.extract_ppn(1),
            self.extract_ppn(0),
            EntryFlags::from_bits_truncate(self.0)
        ))
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
pub struct Table {
    entries: [Entry; ENTRIES_PER_PAGE],
}
const_assert_eq!(size_of::<Table>(), PAGE_SIZE);

unsafe impl Send for Table {}

impl Table {
    pub const fn new() -> Self {
        Table {
            entries: [Entry::new(0); ENTRIES_PER_PAGE],
        }
    }
}

unsafe fn alloc_entry_page(entry: &mut Entry) {
    // println!("[alloc_entry_page] entry {:?} not valid, allocating new page...", entry);
    let ptr = alloc_zeroed(Layout::new::<Table>());
    // println!("[alloc_entry_page] allocated new page for entry at {:?}", ptr);

    entry.set_ppn(PhysicalAddr::new(ptr as u64));
    entry.set_valid();
}

///       The bits should contain only the following:
///          Read, Write, Execute, User, and/or Global
///       The bits MUST include one or more of the following:
///          Read, Write, Execute
pub unsafe fn map(root: *mut Table, vaddr: VirtualAddr, paddr: PhysicalAddr, flags: u64) {
    let mut table = root;
    for lvl in (1..=2).rev() {
        let entry = &mut (*table).entries[vaddr.extract_vpn(lvl)];
        if !entry.is_valid() {
            alloc_entry_page(entry);
        }

        let ppn = entry.extract_ppn_all();
        table = PhysicalAddr::from(ppn, 0).as_mut_ptr::<Table>();
    }

    let entry = &mut (*table).entries[vaddr.extract_vpn(0)];
    entry.set_ppn(paddr);
    entry.set_flags(flags);
    entry.set_valid();

    let mapped = virt_to_phys(root, vaddr);
    assert!(
        mapped == Some(paddr),
        "expect {:?} mapped to {:?} but get {:?}",
        vaddr,
        paddr,
        mapped
    );
}

pub unsafe fn unmap(root: *mut Table) {
    for entry in (*root).entries.iter_mut() {
        let ppn = entry.extract_ppn_all();
        if entry.is_valid() {
            if entry.is_branch() {
                let table = PhysicalAddr::from(ppn, 0).as_mut_ptr::<Table>();
                unmap(table);
            }
            dealloc(
                PhysicalAddr::from(ppn, 0).as_mut_ptr::<u8>(),
                Layout::new::<Table>(),
            );
        }
    }
}

pub fn virt_to_phys(root: *const Table, vaddr: VirtualAddr) -> Option<PhysicalAddr> {
    let mut table = root;
    for lvl in (1..=2).rev() {
        let entry = unsafe { &(*table).entries[vaddr.extract_vpn(lvl)] };
        if !entry.is_valid() {
            return None;
        }
        let ppn = entry.extract_ppn_all();
        table = PhysicalAddr::from(ppn, 0).as_mut_ptr::<Table>();
    }

    let entry = unsafe { &(*table).entries[vaddr.extract_vpn(0)] };
    let ppn = entry.extract_ppn_all();
    Some(PhysicalAddr::from(ppn, vaddr.extract_offset()))
}
