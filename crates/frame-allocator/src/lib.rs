//! Allocation-free buddy management for numeric physical frames.
//!
//! The allocator uses a bitmap supplied by its caller and depends only on
//! `core`. It neither dereferences nor zeroes managed frames, so it makes no
//! assumptions about physical-memory mappings. Synchronization, zeroing, and
//! exclusion of the metadata storage from the managed range are caller policy.

#![no_std]

mod allocator;
mod bitmap;
mod range;

pub use allocator::{
    DeallocationError, FrameAllocator, FrameBlock, InitError, MetadataError, MetadataLayout,
    ReserveError, metadata_layout,
};
pub use range::{FrameRange, RangeError};
