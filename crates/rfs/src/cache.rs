//! A bounded write-back block cache.
//!
//! Dirty blocks reach the device on eviction, explicit synchronization, or
//! cache drop. Until then, the device may contain stale data.

use alloc::collections::VecDeque;
use alloc::sync::Arc;

use bytemuck::Pod;
use spin::Mutex;

use blockdev::{BLOCK_SIZE, BlockDevice};

const CACHE_CAPACITY: usize = 32;

/// Eight-byte alignment permits the typed views used by the on-disk format.
#[repr(C, align(8))]
struct AlignedBlock([u8; BLOCK_SIZE]);

pub struct BlockCache {
    data: AlignedBlock,
    block_id: usize,
    device: Arc<dyn BlockDevice>,
    dirty: bool,
}

impl BlockCache {
    fn load(block_id: usize, device: Arc<dyn BlockDevice>) -> Self {
        let mut data = AlignedBlock([0u8; BLOCK_SIZE]);
        device.read_block(block_id, &mut data.0);
        Self { data, block_id, device, dirty: false }
    }

    /// Views the `T: Pod` at `offset`.
    pub fn read<T: Pod, V>(&self, offset: usize, f: impl FnOnce(&T) -> V) -> V {
        let end = offset + core::mem::size_of::<T>();
        assert!(end <= BLOCK_SIZE, "typed read at offset {offset} overflows block");
        f(bytemuck::from_bytes::<T>(&self.data.0[offset..end]))
    }

    /// Mutates the `T: Pod` at `offset` and marks the block dirty.
    pub fn modify<T: Pod, V>(&mut self, offset: usize, f: impl FnOnce(&mut T) -> V) -> V {
        let end = offset + core::mem::size_of::<T>();
        assert!(end <= BLOCK_SIZE, "typed write at offset {offset} overflows block");
        self.dirty = true;
        f(bytemuck::from_bytes_mut::<T>(&mut self.data.0[offset..end]))
    }

    /// Flushes this block if dirty.
    pub fn sync(&mut self) {
        if self.dirty {
            self.device.write_block(self.block_id, &self.data.0);
            self.dirty = false;
        }
    }
}

impl Drop for BlockCache {
    fn drop(&mut self) { self.sync(); }
}

/// A bounded, write-back cache over one block device.
pub struct BlockCacheManager {
    device: Arc<dyn BlockDevice>,
    resident: Mutex<VecDeque<(usize, Arc<Mutex<BlockCache>>)>>,
}

impl BlockCacheManager {
    pub fn new(device: Arc<dyn BlockDevice>) -> Self {
        Self { device, resident: Mutex::new(VecDeque::new()) }
    }

    /// Returns a cached block, loading it if necessary.
    pub fn get(&self, block_id: usize) -> Arc<Mutex<BlockCache>> {
        let mut resident = self.resident.lock();
        if let Some((_, cache)) = resident.iter().find(|(id, _)| *id == block_id) {
            return cache.clone();
        }
        if resident.len() >= CACHE_CAPACITY {
            let victim = resident
                .iter()
                .position(|(_, cache)| Arc::strong_count(cache) == 1)
                .expect("block cache exhausted: too many blocks held at once");
            resident.remove(victim);
        }
        let cache = Arc::new(Mutex::new(BlockCache::load(block_id, self.device.clone())));
        resident.push_back((block_id, cache.clone()));
        cache
    }

    /// Flushes every dirty resident block to the device.
    pub fn sync_all(&self) {
        let resident = self.resident.lock();
        for (_, cache) in resident.iter() {
            cache.lock().sync();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::BlockCacheManager;
    use alloc::sync::Arc;
    use blockdev::{BLOCK_SIZE, BlockDevice, RamDisk};
    use pretty_assertions::assert_eq;

    fn u32_at(dev: &dyn BlockDevice, block: usize) -> u32 {
        let mut buf = [0u8; BLOCK_SIZE];
        dev.read_block(block, &mut buf);
        u32::from_le_bytes(buf[0..4].try_into().unwrap())
    }

    #[test]
    fn write_back_reaches_disk_only_after_sync() {
        let ram = Arc::new(RamDisk::new(16));
        let mgr = BlockCacheManager::new(ram.clone());

        mgr.get(5).lock().modify(0, |v: &mut u32| *v = 0xDEAD_BEEF);
        assert_eq!(u32_at(ram.as_ref(), 5), 0, "dirty block must not hit disk before sync");

        mgr.sync_all();
        assert_eq!(u32_at(ram.as_ref(), 5), 0xDEAD_BEEF, "sync flushes the dirty block");
    }

    #[test]
    fn reads_are_served_and_persist_across_teardown() {
        let ram = Arc::new(RamDisk::new(16));
        {
            let mgr = BlockCacheManager::new(ram.clone());
            mgr.get(3).lock().modify(0, |v: &mut u32| *v = 42);
            mgr.sync_all();
        }
        let mgr = BlockCacheManager::new(ram.clone());
        let got = mgr.get(3).lock().read(0, |v: &u32| *v);
        assert_eq!(got, 42, "value survives cache teardown via the device");
    }

    #[test]
    fn clean_blocks_are_never_written() {
        let ram = Arc::new(RamDisk::new(16));
        let mgr = BlockCacheManager::new(ram.clone());

        for id in 0..8 {
            mgr.get(id).lock().read(0, |_v: &u32| ());
        }
        mgr.sync_all();
        assert_eq!(ram.writes(), 0, "reading blocks must not write them");

        mgr.get(2).lock().modify(0, |v: &mut u32| *v = 7);
        mgr.sync_all();
        assert_eq!(ram.writes(), 1, "exactly one dirty block flushed");

        mgr.sync_all();
        assert_eq!(ram.writes(), 1, "a second sync writes nothing when nothing is dirty");
    }

    #[test]
    fn eviction_flushes_dirty_blocks() {
        let ram = Arc::new(RamDisk::new(128));
        let mgr = BlockCacheManager::new(ram.clone());

        for id in 0..100usize {
            mgr.get(id).lock().modify(0, |v: &mut u32| *v = id as u32 + 1);
        }

        assert_eq!(u32_at(ram.as_ref(), 0), 1, "evicted block was flushed");
        assert_eq!(u32_at(ram.as_ref(), 99), 0, "resident block not yet flushed");

        mgr.sync_all();
        assert_eq!(u32_at(ram.as_ref(), 99), 100, "sync flushes the stragglers");
    }
}
