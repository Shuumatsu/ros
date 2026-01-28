//! Page table type for Sv39 paging.

use core::mem::size_of;

use super::entry::Entry;
use super::{ENTRIES_PER_PAGE, PAGE_SIZE};

#[derive(Debug)]
#[repr(transparent)]
pub struct Table {
    pub entries: [Entry; ENTRIES_PER_PAGE],
}
const_assert_eq!(size_of::<Table>(), PAGE_SIZE);

unsafe impl Send for Table {}

impl Table {
    pub const fn new() -> Self { Table { entries: [Entry::new(0); ENTRIES_PER_PAGE] } }
}
