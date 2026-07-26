//! rfs — a small, self-contained Unix-style filesystem.
//!
//! rfs is deliberately independent of the kernel: it speaks only to the
//! [`BlockDevice`](blockdev::BlockDevice) trait, carries no architecture- or
//! hardware-specific code, and is fully exercised by host unit tests
//! (`cargo test -p rfs`). The kernel consumes it as a plain dependency and hands
//! it a device backed by the virtio-blk driver; the host `mkfs` tool hands it a
//! [`RamDisk`](blockdev::RamDisk) it later dumps to a file. The on-disk format
//! is written exactly once — here — and every backend agrees by construction.
//!
//! The layers, bottom to top:
//!
//!  * [`blockdev`] (separate crate) — the `BlockDevice` storage contract plus a
//!    RAM-backed reference device.
//!  * [`cache`] — a write-back block cache; everything above talks to this.
//!  * [`bitmap`] — allocation bitmaps for inodes and data blocks.
//!  * [`layout`] — the on-disk data structures.
//!
//! See `DESIGN.md` for the mental model. rfs is `#![no_std]` (+ `alloc`)
//! throughout and carries no `std`-only surface.

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
