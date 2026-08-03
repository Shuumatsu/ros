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

// `not(test)` as well as the feature: the host test modules use `Box` and `Vec` for
// their arenas, so a `cargo test -p paging` without `--features std` failed to compile
// (E0425/E0433) rather than running anything. That is the invocation
// `.cargo/config.toml` documents — it explains at length why no global build target is
// set, precisely so this command works — so the 43 tests in this crate were only
// reachable by someone who already knew to pass a feature flag nothing mentions.
#![cfg_attr(all(not(feature = "std"), not(test)), no_std)]

#[macro_use]
extern crate static_assertions;

pub mod satp;
pub mod sv39;
pub mod utils;

pub use satp::{Mode, Satp};
pub use sv39::access::{Identity, LinearOffset, PhysAccess};
pub use sv39::addr::{MemoryAddr, PhysicalAddr, VirtualAddr};
pub use sv39::entry::{Entry, PteFlags};
pub use sv39::frames::FrameSource;
pub use sv39::mapper::{MapError, Mapper};
pub use sv39::table::Table;
