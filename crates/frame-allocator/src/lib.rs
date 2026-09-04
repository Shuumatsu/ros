//! Allocation-free buddy management for numeric physical frames.
//!
//! The caller supplies bitmap storage and handles synchronization and zeroing.

#![no_std]

mod allocator;
mod bitmap;
mod range;

pub use allocator::{
    DeallocationError, FrameAllocator, FrameBlock, InitError, MetadataError, MetadataLayout,
    ReserveError, metadata_layout,
};
pub use range::{FrameRange, RangeError};
