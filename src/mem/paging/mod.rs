pub mod entry;
pub mod physical_addr;
pub mod virtual_addr;

use core::ptr;
use lazy_static::lazy_static;
use spin::Mutex;

use self::entry::*;
use self::physical_addr::*;
use self::virtual_addr::*;

pub const PAGE_SIZE: usize = 4096;
pub const ENTRY_SIZE: usize = 8;
pub const ENTRIES_PER_PAGE: usize = PAGE_SIZE / ENTRY_SIZE;

#[repr(transparent)]
pub struct Table([Entry; ENTRIES_PER_PAGE]);

impl Table {}

lazy_static! {
    pub static ref ROOT_TABLE: Mutex<Table> = Mutex::new(Table([Entry::new(0); ENTRIES_PER_PAGE]));
}

fn mappages(table: &mut Table, va: VirtualAddr, pa: PhysicalAddr, sz: usize, perm: EntryFlags) {}

fn walk(t2: &mut Table, va: VirtualAddr, alloc: bool) -> &Entry {
    // let pte2 = t2[extract_vpn2!(va)];
    // let t1 = unsafe { *(extract_ppn!(pte2) as *mut Table) };
    panic!()
}
