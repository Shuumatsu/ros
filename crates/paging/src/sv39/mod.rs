//! Sv39 paging mode implementation.
//!
//! Sv39 uses a 39-bit virtual address space with three levels of page tables.

pub mod addr;
pub mod entry;
pub mod table;

pub use addr::{MemoryAddr, PhysicalAddr, VirtualAddr};
pub use entry::*;
pub use table::Table;

use crate::utils::KILOBYTE;

pub const PAGE_SIZE: usize = 4 * KILOBYTE;
pub const ENTRY_SIZE: usize = core::mem::size_of::<u64>();
pub const ENTRIES_PER_PAGE: usize = PAGE_SIZE / ENTRY_SIZE;
