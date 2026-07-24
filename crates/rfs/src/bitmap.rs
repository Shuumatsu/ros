//! Allocation bitmaps.
//!
//! Two regions of the disk — the inode bitmap and the data bitmap — are just
//! runs of bits: a set bit means "that object is in use" (see `DESIGN.md` §7).
//! This type manages one such run. Allocation is "find the lowest clear bit, set
//! it"; freeing is "clear it".
//!
//! Nothing here knows whether a bit stands for an inode or a data block — it
//! returns a plain bit index in `[0, capacity)`, and the caller maps that onto
//! an inode number or a data-block number.

use crate::cache::BlockCacheManager;
use blockdev::BLOCK_SIZE;

/// A block seen as words, for scanning 64 bits at a time.
type BitmapBlock = [u64; BLOCK_SIZE / core::mem::size_of::<u64>()];

const_assert_eq!(core::mem::size_of::<BitmapBlock>(), BLOCK_SIZE);

/// Bits addressable within a single block (`512 * 8 = 4096`).
pub const BITS_PER_BLOCK: usize = BLOCK_SIZE * 8;

/// A bitmap occupying `blocks` consecutive blocks starting at `start_block`.
pub struct Bitmap {
    start_block: usize,
    blocks: usize,
}

impl Bitmap {
    /// A bitmap over the block range `[start_block, start_block + blocks)`.
    pub fn new(start_block: usize, blocks: usize) -> Self { Self { start_block, blocks } }

    /// Greatest number of objects this bitmap can track.
    pub fn capacity(&self) -> usize { self.blocks * BITS_PER_BLOCK }

    /// Allocate the lowest free bit and return its index, or `None` if full.
    pub fn alloc(&self, cache: &BlockCacheManager) -> Option<usize> {
        for block in 0..self.blocks {
            let handle = cache.get(self.start_block + block);
            let mut guard = handle.lock();

            // Find the lowest clear bit under the lock, then set it under the
            // same lock — no window for another allocator to grab it. Scanning
            // via `read` (not `modify`) keeps a full, unchanged block clean.
            let slot = guard.read(0, |map: &BitmapBlock| {
                map.iter().enumerate().find_map(|(word_idx, &word)| {
                    (word != u64::MAX).then(|| (word_idx, word.trailing_ones() as usize))
                })
            });

            if let Some((word_idx, bit)) = slot {
                guard.modify(0, |map: &mut BitmapBlock| map[word_idx] |= 1u64 << bit);
                return Some(block * BITS_PER_BLOCK + word_idx * 64 + bit);
            }
        }
        None
    }

    /// Free bit `index`. Panics on a double free — freeing an already-clear bit
    /// is a filesystem bug, and silently ignoring it would hide corruption.
    pub fn dealloc(&self, cache: &BlockCacheManager, index: usize) {
        let (block, word, bit) = self.locate(index);
        let handle = cache.get(self.start_block + block);
        handle.lock().modify(0, |map: &mut BitmapBlock| {
            let mask = 1u64 << bit;
            assert!(map[word] & mask != 0, "bitmap double free at index {index}");
            map[word] &= !mask;
        });
    }

    /// Whether bit `index` is currently set. For tests and consistency checks.
    pub fn is_allocated(&self, cache: &BlockCacheManager, index: usize) -> bool {
        let (block, word, bit) = self.locate(index);
        let handle = cache.get(self.start_block + block);
        handle.lock().read(0, |map: &BitmapBlock| map[word] & (1u64 << bit) != 0)
    }

    /// Split a bit index into (block within the bitmap, word within block, bit).
    fn locate(&self, index: usize) -> (usize, usize, usize) {
        let block = index / BITS_PER_BLOCK;
        let within = index % BITS_PER_BLOCK;
        assert!(block < self.blocks, "bitmap index {index} out of range");
        (block, within / 64, within % 64)
    }
}

#[cfg(test)]
mod tests {
    use super::{BITS_PER_BLOCK, Bitmap};
    use crate::cache::BlockCacheManager;
    use alloc::sync::Arc;
    use blockdev::RamDisk;
    use pretty_assertions::assert_eq;

    /// A cache over a fresh disk plus a bitmap of `bitmap_blocks`, starting at
    /// block 1 (block 0 stands in for a superblock we do not touch here).
    fn fixture(bitmap_blocks: usize) -> (BlockCacheManager, Bitmap) {
        let ram = Arc::new(RamDisk::new(1 + bitmap_blocks));
        (BlockCacheManager::new(ram), Bitmap::new(1, bitmap_blocks))
    }

    #[test]
    fn allocates_lowest_free_in_order() {
        let (mgr, bm) = fixture(1);
        for expect in 0..8 {
            assert_eq!(bm.alloc(&mgr), Some(expect), "sequential allocation");
        }
    }

    #[test]
    fn dealloc_frees_for_reuse() {
        let (mgr, bm) = fixture(1);
        let (a, b, c) = (bm.alloc(&mgr), bm.alloc(&mgr), bm.alloc(&mgr));
        assert_eq!((a, b, c), (Some(0), Some(1), Some(2)));

        bm.dealloc(&mgr, 1);
        assert_eq!(bm.alloc(&mgr), Some(1), "lowest freed bit is reused first");
        assert_eq!(bm.alloc(&mgr), Some(3), "then allocation continues past the high-water mark");
    }

    #[test]
    fn is_allocated_tracks_state() {
        let (mgr, bm) = fixture(1);
        let bit = bm.alloc(&mgr).unwrap();
        assert!(bm.is_allocated(&mgr, bit), "just-allocated bit reads as set");
        bm.dealloc(&mgr, bit);
        assert!(!bm.is_allocated(&mgr, bit), "freed bit reads as clear");
    }

    #[test]
    fn fills_then_returns_none() {
        let (mgr, bm) = fixture(1);
        assert_eq!(bm.capacity(), BITS_PER_BLOCK);
        for _ in 0..BITS_PER_BLOCK {
            assert!(bm.alloc(&mgr).is_some());
        }
        assert_eq!(bm.alloc(&mgr), None, "a full bitmap yields None");
    }

    #[test]
    fn allocation_crosses_block_boundary() {
        let (mgr, bm) = fixture(2);
        assert_eq!(bm.capacity(), 2 * BITS_PER_BLOCK);
        for _ in 0..BITS_PER_BLOCK {
            bm.alloc(&mgr).unwrap();
        }
        assert_eq!(bm.alloc(&mgr), Some(BITS_PER_BLOCK), "spills into the second bitmap block");
    }

    #[test]
    fn survives_reopen_via_device() {
        let ram = Arc::new(RamDisk::new(2));
        {
            let mgr = BlockCacheManager::new(ram.clone());
            let bm = Bitmap::new(1, 1);
            assert_eq!(bm.alloc(&mgr), Some(0));
            assert_eq!(bm.alloc(&mgr), Some(1));
            mgr.sync_all();
        }
        // Reopen over the same disk: the allocation state is still there.
        let mgr = BlockCacheManager::new(ram.clone());
        let bm = Bitmap::new(1, 1);
        assert!(bm.is_allocated(&mgr, 0));
        assert!(bm.is_allocated(&mgr, 1));
        assert_eq!(bm.alloc(&mgr), Some(2), "allocation continues after the persisted bits");
    }

    #[test]
    #[should_panic(expected = "double free")]
    fn double_free_panics() {
        let (mgr, bm) = fixture(1);
        let bit = bm.alloc(&mgr).unwrap();
        bm.dealloc(&mgr, bit);
        bm.dealloc(&mgr, bit);
    }
}
