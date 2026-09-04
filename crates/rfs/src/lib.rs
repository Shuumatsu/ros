//! A `no_std` filesystem over the [`BlockDevice`](blockdev::BlockDevice) interface.
//!
//! [`layout`] defines the persistent format; [`cache`] provides write-back storage.

#![no_std]

extern crate alloc;

#[macro_use]
extern crate static_assertions;

pub mod bitmap;
pub mod cache;
mod dir;
pub mod file;
pub mod fs;
pub mod layout;

#[cfg(test)]
mod test_support;

pub use bitmap::{BITS_PER_BLOCK, Bitmap};
pub use cache::{BlockCache, BlockCacheManager};
pub use file::File;
pub use fs::Fs;
pub use layout::{
    DIRECT_COUNT, DirEntry, DiskInode, FS_MAGIC, INODES_PER_BLOCK, InodeType, MAX_FILE_BLOCKS,
    MAX_FILE_SIZE, NAME_MAX, POINTERS_PER_BLOCK, ROOT_INODE, SuperBlock,
};
