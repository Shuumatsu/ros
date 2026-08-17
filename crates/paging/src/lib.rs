//! RISC-V paging structures, independent of both the platform and the translation scheme.
//!
//! All code is testable on the host machine without hardware dependencies.
//!
//! # One walk, three schemes
//!
//! Sv39, Sv48 and Sv57 share a page size, a page-table shape, a PTE format and a walk;
//! they differ only in how many levels that walk descends. So the difference is one
//! associated const on [`Scheme`], and everything else in the crate — [`Table`], [`Entry`],
//! [`Mapper`] — is written once and parameterised by it. [`geometry`] holds what all three
//! share; a scheme holds what it does not.
//!
//! Nothing outside [`Mapper`] and the two [`Table`] builders takes a scheme at all: an
//! address, a PTE and a page size mean the same thing under each, which is why the rest of
//! a kernel can depend on this crate without naming one.
//!
//! # No allocator, no addressing model
//!
//! The crate never allocates and never assumes how physical memory is reached.
//! Both are caller policy, injected as [`FrameSource`] (where intermediate page
//! tables come from) and [`PhysAccess`] (how to turn a physical address into a
//! usable pointer). That keeps it usable from early boot — before any allocator
//! exists — and correct for a higher-half kernel, where a physical address is
//! *not* a valid pointer.

// `not(test)` as well as the feature: the host test modules use `Box` and `Vec` for
// their arenas, so without it a plain `cargo test -p paging` fails to compile
// (E0425/E0433) instead of running. That is the invocation `.cargo/config.toml`
// documents, and it must work without an extra feature flag.
#![cfg_attr(all(not(feature = "std"), not(test)), no_std)]

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
pub mod utils;

pub use access::{Identity, LinearOffset, PhysAccess};
pub use addr::{MemoryAddr, PhysicalAddr, VirtualAddr};
pub use frames::FrameSource;
pub use geometry::{
    ENTRIES_PER_PAGE, ENTRY_SIZE, GIGAPAGE, MAX_LEVELS, PAGE_OFFSET_BITS, PAGE_SIZE, PPN_BITS,
    ROOT_ENTRIES_PER_HALF, SUPERPAGE, VPN_BITS, page_size_at,
};
pub use mapper::{MapError, Mapper, Unmapped};
pub use pte::{Entry, PteFlags};
pub use satp::{Mode, Satp};
pub use scheme::{Scheme, Sv39, Sv48, Sv57};
pub use table::Table;
