//! RV64 page tables, address types, and `satp` encoding for Sv39, Sv48, and Sv57.
//!
//! Frame allocation and access to physical memory are supplied through
//! [`FrameSource`] and [`PhysAccess`].

#![cfg_attr(not(test), no_std)]

#[macro_use]
extern crate static_assertions;

pub mod access;
pub mod addr;
pub mod frames;
pub mod geometry;
pub mod mapper;
pub mod pte;
pub mod satp;
pub mod scheme;
pub mod table;
pub(crate) mod utils;

pub use access::{LinearOffset, PhysAccess};
pub use addr::{MemoryAddr, PhysicalAddr, VirtualAddr};
pub use frames::FrameSource;
pub use geometry::{
    ENTRIES_PER_PAGE, ENTRY_SIZE, GIGAPAGE, MAX_LEVELS, PAGE_OFFSET_BITS, PAGE_SIZE, PPN_BITS,
    ROOT_ENTRIES_PER_HALF, SUPERPAGE, VPN_BITS, page_size_at,
};
pub use mapper::{MapError, Mapper, Unmapped};
pub use pte::{Entry, PteFlags};
pub use satp::{Mode, Satp};
pub use scheme::{Scheme, Sv39, Sv48, Sv57, vpn};
pub use table::Table;
