//! The block-device layer: the storage contract the filesystem sits on, plus a
//! RAM-backed reference implementation.
//!
//! Deliberately tiny and filesystem-agnostic. `rfs` depends on it for the
//! [`BlockDevice`] trait; the kernel's virtio-blk driver implements the same
//! trait; the `mkfs` host tool uses [`RamDisk`]. Nothing here knows about inodes
//! or directories — it only moves fixed-size blocks. Keeping it separate means a
//! block driver never has to depend on the filesystem.

#![no_std]

extern crate alloc;

mod device;
mod ramdisk;

pub use device::{BLOCK_SIZE, BlockDevice};
pub use ramdisk::RamDisk;
