//! Filesystem-independent block I/O and a RAM-backed implementation.

#![no_std]

extern crate alloc;

mod device;
mod ramdisk;

pub use device::{BLOCK_SIZE, BlockDevice};
pub use ramdisk::RamDisk;
