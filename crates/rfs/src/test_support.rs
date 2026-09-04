use alloc::sync::Arc;
use alloc::vec::Vec;

use blockdev::RamDisk;

use crate::cache::BlockCacheManager;
use crate::fs::Fs;

/// Test geometry large enough for double-indirect files.
pub const TEST_BLOCKS: usize = 8192;
pub const TEST_NINODES: usize = 512;

pub fn test_ram() -> Arc<RamDisk> { Arc::new(RamDisk::new(TEST_BLOCKS)) }

pub fn format_on(ram: &Arc<RamDisk>) -> Fs {
    Fs::format(Arc::new(BlockCacheManager::new(ram.clone())), TEST_BLOCKS, TEST_NINODES)
}

pub fn mount_on(ram: &Arc<RamDisk>) -> Fs {
    Fs::mount(Arc::new(BlockCacheManager::new(ram.clone())))
}

pub fn fresh() -> Fs { format_on(&test_ram()) }

pub fn pattern(len: usize) -> Vec<u8> { (0..len).map(|i| (i % 251) as u8).collect() }
