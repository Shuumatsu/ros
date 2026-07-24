//! A small write-back block cache.
//!
//! Everything above this file reads and writes whole blocks *through* the cache,
//! never straight to the device (see `DESIGN.md` §9). The rules the rest of the
//! filesystem relies on:
//!
//!  * A block is read from the device at most once, then served from RAM.
//!  * A modification marks the block **dirty** but does **not** touch the disk.
//!  * A dirty block reaches the disk only when it is evicted, when
//!    [`BlockCacheManager::sync_all`] is called, or when the block is dropped.
//!
//! So until you sync, the copy in RAM is the truth and the disk is stale.
//! Forgetting to sync loses data — that is the deal write-back makes in exchange
//! for coalescing repeated writes to the same block.

use alloc::collections::VecDeque;
use alloc::sync::Arc;

use bytemuck::Pod;
use spin::Mutex;

use blockdev::{BLOCK_SIZE, BlockDevice};

/// Upper bound on blocks kept resident at once. Loading one more evicts the
/// oldest block no one is currently holding (flushing it first if dirty). Big
/// enough that ordinary bitmap/inode/directory work never pins them all.
const CACHE_CAPACITY: usize = 32;

/// A block's worth of bytes, aligned so typed views taken through bytemuck
/// (`&DiskInode`, `&[u64; 64]`, …) are correctly aligned. Every on-disk record
/// sits at a natural, size-aligned offset within its block, so an 8-aligned base
/// makes every such view aligned.
#[repr(C, align(8))]
struct AlignedBlock([u8; BLOCK_SIZE]);

/// One cached block: its bytes in RAM, whether they differ from disk, and the
/// device to flush back to.
pub struct BlockCache {
    data: AlignedBlock,
    block_id: usize,
    device: Arc<dyn BlockDevice>,
    dirty: bool,
}

impl BlockCache {
    /// Read `block_id` off the device into a fresh, clean cache entry.
    fn load(block_id: usize, device: Arc<dyn BlockDevice>) -> Self {
        let mut data = AlignedBlock([0u8; BLOCK_SIZE]);
        device.read_block(block_id, &mut data.0);
        Self { data, block_id, device, dirty: false }
    }

    /// View the `T` stored at `offset` and pass it to `f`. `T: Pod` guarantees
    /// every bit pattern is a valid `T`, so reinterpreting on-disk bytes is
    /// sound — which is exactly why `DiskInode::type_` is a raw `u32`, not an
    /// enum (a corrupt block would otherwise be an invalid discriminant, UB).
    pub fn read<T: Pod, V>(&self, offset: usize, f: impl FnOnce(&T) -> V) -> V {
        let end = offset + core::mem::size_of::<T>();
        assert!(end <= BLOCK_SIZE, "typed read at offset {offset} overflows block");
        f(bytemuck::from_bytes::<T>(&self.data.0[offset..end]))
    }

    /// Mutate the `T` stored at `offset` in place through `f`, marking the block
    /// dirty. The change stays in RAM until the block is synced or evicted.
    pub fn modify<T: Pod, V>(&mut self, offset: usize, f: impl FnOnce(&mut T) -> V) -> V {
        let end = offset + core::mem::size_of::<T>();
        assert!(end <= BLOCK_SIZE, "typed write at offset {offset} overflows block");
        self.dirty = true;
        f(bytemuck::from_bytes_mut::<T>(&mut self.data.0[offset..end]))
    }

    /// Flush to disk if dirty; a no-op otherwise. Idempotent.
    pub fn sync(&mut self) {
        if self.dirty {
            self.device.write_block(self.block_id, &self.data.0);
            self.dirty = false;
        }
    }
}

impl Drop for BlockCache {
    /// A block flushes itself when dropped, so eviction and teardown never lose
    /// dirty data even without an explicit sync.
    fn drop(&mut self) { self.sync(); }
}

/// A bounded, write-back cache over one block device.
///
/// Shared as `Arc<BlockCacheManager>`: methods take `&self` and lock internally,
/// so bitmaps, inodes and directories all reach blocks through one handle.
/// Blocks are keyed by id; two managers over different devices never share
/// entries (unlike a global cache would), which keeps host tests isolated.
pub struct BlockCacheManager {
    device: Arc<dyn BlockDevice>,
    resident: Mutex<VecDeque<(usize, Arc<Mutex<BlockCache>>)>>,
}

impl BlockCacheManager {
    /// Wrap `device` in an empty cache.
    pub fn new(device: Arc<dyn BlockDevice>) -> Self {
        Self { device, resident: Mutex::new(VecDeque::new()) }
    }

    /// Get block `block_id`, loading it from disk if not resident. Lock the
    /// returned handle to [`read`](BlockCache::read) or
    /// [`modify`](BlockCache::modify) it.
    pub fn get(&self, block_id: usize) -> Arc<Mutex<BlockCache>> {
        let mut resident = self.resident.lock();
        if let Some((_, cache)) = resident.iter().find(|(id, _)| *id == block_id) {
            return cache.clone();
        }
        if resident.len() >= CACHE_CAPACITY {
            // Evict the oldest block nobody else is holding; its `Drop` flushes
            // it if dirty. If every resident block is pinned we cannot make
            // room — that means a caller is holding too many blocks at once.
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

    /// Flush every dirty resident block to disk. Write-back means nothing is
    /// durable until this (or eviction, or drop) runs — call it before handing
    /// the disk to anyone else.
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
        // Still in RAM only — the disk is untouched.
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
        // A fresh cache over the same device sees the persisted value.
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

        // Touch far more blocks than the cache holds, keeping no handles, so the
        // oldest are evicted — and must be flushed on the way out.
        for id in 0..100usize {
            mgr.get(id).lock().modify(0, |v: &mut u32| *v = id as u32 + 1);
        }

        // Block 0 is long evicted, hence already on disk...
        assert_eq!(u32_at(ram.as_ref(), 0), 1, "evicted block was flushed");
        // ...while the most recent block is still resident and unflushed.
        assert_eq!(u32_at(ram.as_ref(), 99), 0, "resident block not yet flushed");

        mgr.sync_all();
        assert_eq!(u32_at(ram.as_ref(), 99), 100, "sync flushes the stragglers");
    }
}
