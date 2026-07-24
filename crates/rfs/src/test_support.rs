//! Shared helpers for the crate's unit tests — one source of truth for building
//! a formatted in-memory filesystem and for test data, so no test module has to
//! reinvent them.

use alloc::sync::Arc;
use alloc::vec::Vec;

use blockdev::RamDisk;

use crate::cache::BlockCacheManager;
use crate::fs::Fs;

/// Standard test disk geometry: big enough to exercise double-indirect files.
pub const TEST_BLOCKS: usize = 8192;
pub const TEST_NINODES: usize = 512;

/// A fresh in-memory disk of the standard test size.
pub fn test_ram() -> Arc<RamDisk> { Arc::new(RamDisk::new(TEST_BLOCKS)) }

/// Format `ram` as an rfs filesystem (for tests that later remount the disk).
pub fn format_on(ram: &Arc<RamDisk>) -> Fs {
    Fs::format(Arc::new(BlockCacheManager::new(ram.clone())), TEST_BLOCKS, TEST_NINODES)
}

/// Mount an already-formatted `ram`.
pub fn mount_on(ram: &Arc<RamDisk>) -> Fs {
    Fs::mount(Arc::new(BlockCacheManager::new(ram.clone())))
}

/// A fresh formatted filesystem on its own private disk.
pub fn fresh() -> Fs { format_on(&test_ram()) }

/// A deterministic byte pattern; read-back mismatches are then obvious.
pub fn pattern(len: usize) -> Vec<u8> { (0..len).map(|i| (i % 251) as u8).collect() }
