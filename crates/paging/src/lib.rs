//! Platform-agnostic RISC-V paging structures.
//!
//! This crate provides pure data structures and operations for RISC-V
//! paging (Sv39, with future support for Sv48/Sv57). All code is testable
//! on the host machine without hardware dependencies.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

#[macro_use]
extern crate static_assertions;

pub mod sv39;
pub mod utils;

pub use sv39::addr::{MemoryAddr, PhysicalAddr, VirtualAddr};
