//! Platform-agnostic RISC-V paging structures.
//!
//! This crate provides pure data structures and operations for RISC-V
//! paging (Sv39, with future support for Sv48/Sv57). All code is testable
//! on the host machine without hardware dependencies.
//!
//! # No allocator, no addressing model
//!
//! The crate never allocates and never assumes how physical memory is reached.
//! Both are caller policy, injected as [`sv39::FrameSource`] (where
//! intermediate page tables come from) and [`sv39::PhysAccess`] (how to turn a
//! physical address into a usable pointer). That keeps it usable from early
//! boot — before any allocator exists — and correct for a higher-half kernel,
//! where a physical address is *not* a valid pointer.

#![cfg_attr(not(feature = "std"), no_std)]

#[macro_use]
extern crate static_assertions;

pub mod sv39;
pub mod utils;

pub use sv39::access::{Identity, LinearOffset, PhysAccess};
pub use sv39::addr::{MemoryAddr, PhysicalAddr, VirtualAddr};
pub use sv39::entry::{Entry, PteFlags};
pub use sv39::frames::FrameSource;
pub use sv39::mapper::{MapError, Mapper};
pub use sv39::table::Table;
