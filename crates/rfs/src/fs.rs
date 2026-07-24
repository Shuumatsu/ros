//! The filesystem core: on-disk layout math, allocation, and the inode
//! block-map that turns a file byte offset into a data block.
//!
//! [`Fs`] owns the block cache, a cached copy of the [`SuperBlock`], and the two
//! allocation [`Bitmap`]s. Everything here is the machinery behind `DESIGN.md`
//! §4 (block map), §7 (allocation) and §8 (read/write/truncate). Directories and
//! path resolution are built on top of this, in a later layer.
//!
//! ## Concurrency
//! Each block touched is individually locked through the cache, so single block
//! accesses are safe. Compound operations (a `write_at` that grows a file across
//! several blocks and indirect blocks) are **not** atomic; the kernel serializes
//! filesystem access with one outer lock. rfs does not try to be cleverer.

use alloc::sync::Arc;

use bytemuck::Zeroable;

use crate::bitmap::{BITS_PER_BLOCK, Bitmap};
use crate::cache::BlockCacheManager;
use crate::layout::{
    DIRECT_COUNT, DiskInode, FS_MAGIC, INODES_PER_BLOCK, InodeType, MAX_FILE_SIZE,
    POINTERS_PER_BLOCK, ROOT_INODE, SuperBlock,
};
use blockdev::BLOCK_SIZE;

/// Where logical block `inner` of a file lives in the block-map hierarchy.
///
/// The single place that encodes the direct / single-indirect / double-indirect
/// layout (`DESIGN.md` §4): both the read path ([`Fs::block_map`]) and the write
/// path ([`Fs::ensure_block`]) dispatch on it, so the mapping is decided once.
enum BlockSlot {
    /// Inline direct pointer `direct[i]`.
    Direct(usize),
    /// Entry `i` of the single-indirect block.
    SingleIndirect(usize),
    /// Entry `l2` of the `l1`-th block reached through the double-indirect block.
    DoubleIndirect { l1: usize, l2: usize },
}

/// Classify a file-logical block index into its [`BlockSlot`].
fn locate_block(inner: usize) -> BlockSlot {
    if inner < DIRECT_COUNT {
        return BlockSlot::Direct(inner);
    }
    let inner = inner - DIRECT_COUNT;
    if inner < POINTERS_PER_BLOCK {
        return BlockSlot::SingleIndirect(inner);
    }
    let inner = inner - POINTERS_PER_BLOCK;
    BlockSlot::DoubleIndirect { l1: inner / POINTERS_PER_BLOCK, l2: inner % POINTERS_PER_BLOCK }
}

/// A mounted rfs filesystem over one block device.
pub struct Fs {
    cache: Arc<BlockCacheManager>,
    sb: SuperBlock,
    inode_bitmap: Bitmap,
    data_bitmap: Bitmap,
}

impl Fs {
    // ------------------------------------------------------------------ mount

    /// Format `cache`'s device as a fresh rfs image of `total_blocks` blocks
    /// with room for `ninodes` inodes, then mount it. Region sizes are derived
    /// and recorded in the superblock (see `DESIGN.md` §2); the root directory
    /// is created as inode 0.
    pub fn format(cache: Arc<BlockCacheManager>, total_blocks: usize, ninodes: usize) -> Fs {
        let inode_bitmap_len = ninodes.div_ceil(BITS_PER_BLOCK);
        let inode_table_len = ninodes.div_ceil(INODES_PER_BLOCK);

        // What is left after the superblock and inode region is split between the
        // data bitmap and the data blocks it tracks. Each data-bitmap block
        // covers BITS_PER_BLOCK data blocks plus itself, hence the `+ 1`.
        let remaining = total_blocks
            .checked_sub(1 + inode_bitmap_len + inode_table_len)
            .expect("rfs: disk too small for the inode region");
        let data_bitmap_len = remaining.div_ceil(BITS_PER_BLOCK + 1);
        let data_len = remaining - data_bitmap_len;
        assert!(data_len > 0, "rfs: disk too small for any data blocks");

        let inode_bitmap_start = 1;
        let data_bitmap_start = inode_bitmap_start + inode_bitmap_len;
        let inode_table_start = data_bitmap_start + data_bitmap_len;
        let data_start = inode_table_start + inode_table_len;

        let sb = SuperBlock {
            magic: FS_MAGIC,
            block_size: BLOCK_SIZE as u32,
            total_blocks: total_blocks as u32,
            ninodes: ninodes as u32,
            inode_bitmap_start: inode_bitmap_start as u32,
            inode_bitmap_len: inode_bitmap_len as u32,
            data_bitmap_start: data_bitmap_start as u32,
            data_bitmap_len: data_bitmap_len as u32,
            inode_table_start: inode_table_start as u32,
            inode_table_len: inode_table_len as u32,
            data_start: data_start as u32,
            data_len: data_len as u32,
            root_inode: ROOT_INODE,
        };

        // Zero the metadata regions so reformatting a used disk is clean. Block 0
        // (superblock) is written next; data blocks are zeroed lazily on alloc.
        for block in inode_bitmap_start..data_start {
            cache.get(block).lock().modify(0, |blk: &mut [u8; BLOCK_SIZE]| blk.fill(0));
        }
        cache.get(0).lock().modify(0, |dst: &mut SuperBlock| *dst = sb);

        let fs = Fs {
            cache,
            sb,
            inode_bitmap: Bitmap::new(inode_bitmap_start, inode_bitmap_len),
            data_bitmap: Bitmap::new(data_bitmap_start, data_bitmap_len),
        };

        // Root directory: inode 0, empty.
        let root = fs.alloc_inode(InodeType::Dir).expect("rfs: cannot allocate root inode");
        assert_eq!(root, ROOT_INODE, "rfs: root must be inode 0");
        fs.sync();
        fs
    }

    /// Mount an already-formatted device. Panics on a bad magic — an unformatted
    /// or foreign disk is a caller error, not something to limp along with.
    pub fn mount(cache: Arc<BlockCacheManager>) -> Fs {
        let sb = cache.get(0).lock().read(0, |sb: &SuperBlock| *sb);
        assert_eq!(sb.magic, FS_MAGIC, "rfs: bad superblock magic {:#x}", sb.magic);
        let inode_bitmap =
            Bitmap::new(sb.inode_bitmap_start as usize, sb.inode_bitmap_len as usize);
        let data_bitmap = Bitmap::new(sb.data_bitmap_start as usize, sb.data_bitmap_len as usize);
        Fs { cache, sb, inode_bitmap, data_bitmap }
    }

    // --------------------------------------------------------------- accessors

    /// The root directory's inode number.
    pub fn root_inode(&self) -> u32 { self.sb.root_inode }

    /// The mounted superblock.
    pub fn superblock(&self) -> &SuperBlock { &self.sb }

    /// Flush all dirty blocks to disk. Nothing is durable until this runs.
    pub fn sync(&self) { self.cache.sync_all(); }

    /// Byte length of inode `id`.
    pub fn inode_size(&self, id: u32) -> usize { self.read_disk_inode(id, |di| di.size as usize) }

    /// Kind of inode `id`, or `None` if its `type_` is not a known value.
    pub fn inode_type(&self, id: u32) -> Option<InodeType> {
        self.read_disk_inode(id, |di| InodeType::from_raw(di.type_))
    }

    // -------------------------------------------------------- inode allocation

    /// Allocate and initialize a fresh inode of `kind` (size 0, one link), or
    /// `None` if the inode table is full.
    pub fn alloc_inode(&self, kind: InodeType) -> Option<u32> {
        let bit = self.inode_bitmap.alloc(&self.cache)?;
        // The bitmap rounds up to whole blocks, so it may cover more bits than
        // there are inodes; refuse anything past the real count.
        if bit >= self.sb.ninodes as usize {
            self.inode_bitmap.dealloc(&self.cache, bit);
            return None;
        }
        let id = bit as u32;
        self.modify_disk_inode(id, |di| {
            *di = DiskInode::zeroed();
            di.type_ = kind.as_raw();
            di.nlink = 1;
        });
        Some(id)
    }

    /// Free inode `id`: release its data, wipe the record, clear the bitmap bit.
    pub fn free_inode(&self, id: u32) {
        self.truncate(id);
        self.modify_disk_inode(id, |di| *di = DiskInode::zeroed());
        self.inode_bitmap.dealloc(&self.cache, id as usize);
    }

    /// Read inode `id` through `f`.
    pub fn read_disk_inode<V>(&self, id: u32, f: impl FnOnce(&DiskInode) -> V) -> V {
        let (block, offset) = self.inode_pos(id);
        self.cache.get(block).lock().read(offset, f)
    }

    /// Modify inode `id` through `f`.
    pub fn modify_disk_inode<V>(&self, id: u32, f: impl FnOnce(&mut DiskInode) -> V) -> V {
        let (block, offset) = self.inode_pos(id);
        self.cache.get(block).lock().modify(offset, f)
    }

    /// The (block, byte offset) where inode `id` lives in the inode table.
    fn inode_pos(&self, id: u32) -> (usize, usize) {
        let block = self.sb.inode_table_start as usize + id as usize / INODES_PER_BLOCK;
        let offset = (id as usize % INODES_PER_BLOCK) * core::mem::size_of::<DiskInode>();
        (block, offset)
    }

    // --------------------------------------------------------- data allocation

    /// Allocate a zeroed data block, returning its absolute block id, or `None`
    /// if the data region is full. Zeroing matters: indirect blocks rely on
    /// unset pointers reading as 0, and file data must not leak old bytes.
    fn alloc_data_block(&self) -> Option<usize> {
        let bit = self.data_bitmap.alloc(&self.cache)?;
        if bit >= self.sb.data_len as usize {
            self.data_bitmap.dealloc(&self.cache, bit);
            return None;
        }
        let block = self.sb.data_start as usize + bit;
        self.cache.get(block).lock().modify(0, |blk: &mut [u8; BLOCK_SIZE]| blk.fill(0));
        Some(block)
    }

    /// Return data block `block` to the pool.
    fn free_data_block(&self, block: usize) {
        let bit = block - self.sb.data_start as usize;
        self.data_bitmap.dealloc(&self.cache, bit);
    }

    // ------------------------------------------------------------- read/write

    /// Read up to `buf.len()` bytes of inode `id` starting at `offset`, returning
    /// how many were read (0 at or past EOF). Unallocated blocks inside the file
    /// (holes) read as zeros.
    pub fn read_at(&self, id: u32, offset: usize, buf: &mut [u8]) -> usize {
        let di = self.read_disk_inode(id, |di| *di);
        let size = di.size as usize;
        if offset >= size || buf.is_empty() {
            return 0;
        }
        let end = (offset + buf.len()).min(size);
        let mut done = 0;
        let mut pos = offset;
        while pos < end {
            let within = pos % BLOCK_SIZE;
            let n = (BLOCK_SIZE - within).min(end - pos);
            match self.block_map(&di, pos / BLOCK_SIZE) {
                Some(block) => self.cache.get(block).lock().read(0, |blk: &[u8; BLOCK_SIZE]| {
                    buf[done..done + n].copy_from_slice(&blk[within..within + n]);
                }),
                None => buf[done..done + n].fill(0),
            }
            done += n;
            pos += n;
        }
        done
    }

    /// Write `buf` into inode `id` at `offset`, allocating data (and indirect)
    /// blocks as needed and growing the file. Returns bytes written (== buf.len).
    pub fn write_at(&self, id: u32, offset: usize, buf: &[u8]) -> usize {
        if buf.is_empty() {
            return 0;
        }
        let end = offset + buf.len();
        assert!(end <= MAX_FILE_SIZE, "rfs: write to {end} exceeds max file size {MAX_FILE_SIZE}");
        let mut done = 0;
        let mut pos = offset;
        while pos < end {
            let within = pos % BLOCK_SIZE;
            let n = (BLOCK_SIZE - within).min(end - pos);
            let block = self.ensure_block(id, pos / BLOCK_SIZE);
            self.cache.get(block).lock().modify(0, |blk: &mut [u8; BLOCK_SIZE]| {
                blk[within..within + n].copy_from_slice(&buf[done..done + n]);
            });
            done += n;
            pos += n;
        }
        self.modify_disk_inode(id, |di| {
            if end as u32 > di.size {
                di.size = end as u32;
            }
        });
        done
    }

    /// Free every data block of inode `id` and reset it to length 0. The inode
    /// itself remains allocated (see [`free_inode`](Self::free_inode) to reclaim it).
    pub fn truncate(&self, id: u32) {
        let di = self.read_disk_inode(id, |di| *di);
        for &block in &di.direct {
            if block != 0 {
                self.free_data_block(block as usize);
            }
        }
        if di.indirect != 0 {
            self.free_indirect(di.indirect as usize);
        }
        if di.double_indirect != 0 {
            self.free_double_indirect(di.double_indirect as usize);
        }
        self.modify_disk_inode(id, |di| {
            di.size = 0;
            di.direct = [0; DIRECT_COUNT];
            di.indirect = 0;
            di.double_indirect = 0;
        });
    }

    // ------------------------------------------------------ block-map internals

    /// Map logical block `inner` of `di` to a physical block, or `None` if it is
    /// not allocated (a hole). Read path — never allocates.
    fn block_map(&self, di: &DiskInode, inner: usize) -> Option<usize> {
        match locate_block(inner) {
            BlockSlot::Direct(i) => (di.direct[i] != 0).then_some(di.direct[i] as usize),
            BlockSlot::SingleIndirect(i) => match di.indirect {
                0 => None,
                block => self.read_pointer(block as usize, i),
            },
            BlockSlot::DoubleIndirect { l1, l2 } => {
                if di.double_indirect == 0 {
                    return None;
                }
                let mid = self.read_pointer(di.double_indirect as usize, l1)?;
                self.read_pointer(mid, l2)
            }
        }
    }

    /// Ensure logical block `inner` of inode `id` is backed by a physical block,
    /// allocating it (and any missing indirect blocks) on the way. Write path.
    fn ensure_block(&self, id: u32, inner: usize) -> usize {
        match locate_block(inner) {
            BlockSlot::Direct(i) => {
                let existing = self.read_disk_inode(id, |di| di.direct[i]);
                if existing != 0 {
                    return existing as usize;
                }
                let block = self.alloc_data_block().expect("rfs: out of data blocks");
                self.modify_disk_inode(id, |di| di.direct[i] = block as u32);
                block
            }
            BlockSlot::SingleIndirect(i) => {
                let indirect = self.ensure_inode_pointer(id, |di| &mut di.indirect);
                self.ensure_pointer(indirect, i)
            }
            BlockSlot::DoubleIndirect { l1, l2 } => {
                let double = self.ensure_inode_pointer(id, |di| &mut di.double_indirect);
                let mid = self.ensure_pointer(double, l1);
                self.ensure_pointer(mid, l2)
            }
        }
    }

    /// Ensure the inode-level pointer selected by `field` names an allocated
    /// block, allocating a (zeroed) one if it is currently 0. Returns the block.
    fn ensure_inode_pointer(&self, id: u32, field: impl Fn(&mut DiskInode) -> &mut u32) -> usize {
        let existing = self.modify_disk_inode(id, |di| *field(di));
        if existing != 0 {
            return existing as usize;
        }
        let block = self.alloc_data_block().expect("rfs: out of data blocks");
        self.modify_disk_inode(id, |di| *field(di) = block as u32);
        block
    }

    /// Ensure entry `idx` of the indirect block `indirect` names an allocated
    /// block, allocating a (zeroed) one if needed. Returns the pointed-to block.
    fn ensure_pointer(&self, indirect: usize, idx: usize) -> usize {
        let offset = idx * core::mem::size_of::<u32>();
        let existing = self.cache.get(indirect).lock().read(offset, |p: &u32| *p);
        if existing != 0 {
            return existing as usize;
        }
        let block = self.alloc_data_block().expect("rfs: out of data blocks");
        self.cache.get(indirect).lock().modify(offset, |p: &mut u32| *p = block as u32);
        block
    }

    /// Read entry `idx` of indirect block `indirect`, or `None` if it is 0.
    fn read_pointer(&self, indirect: usize, idx: usize) -> Option<usize> {
        let offset = idx * core::mem::size_of::<u32>();
        let p = self.cache.get(indirect).lock().read(offset, |p: &u32| *p);
        (p != 0).then_some(p as usize)
    }

    /// Free every data block an indirect block points to, then the block itself.
    fn free_indirect(&self, block: usize) {
        let pointers = self.cache.get(block).lock().read(0, |p: &[u32; POINTERS_PER_BLOCK]| *p);
        for &p in &pointers {
            if p != 0 {
                self.free_data_block(p as usize);
            }
        }
        self.free_data_block(block);
    }

    /// Free a double-indirect block: every second-level indirect block it names
    /// (and their data), then itself.
    fn free_double_indirect(&self, block: usize) {
        let pointers = self.cache.get(block).lock().read(0, |p: &[u32; POINTERS_PER_BLOCK]| *p);
        for &mid in &pointers {
            if mid != 0 {
                self.free_indirect(mid as usize);
            }
        }
        self.free_data_block(block);
    }

    /// Data blocks currently allocated. Test/consistency helper.
    #[cfg(test)]
    fn used_data_blocks(&self) -> usize {
        (0..self.sb.data_len as usize)
            .filter(|&i| self.data_bitmap.is_allocated(&self.cache, i))
            .count()
    }
}

#[cfg(test)]
mod tests {
    use crate::layout::{FS_MAGIC, InodeType, ROOT_INODE};
    use crate::test_support::{format_on, fresh, mount_on, pattern, test_ram};
    use alloc::vec;
    use pretty_assertions::assert_eq;

    #[test]
    fn format_layout_is_ordered_and_bounded() {
        let fs = fresh();
        let sb = *fs.superblock();
        assert_eq!(sb.magic, FS_MAGIC);
        assert_eq!(sb.inode_bitmap_start, 1, "metadata begins right after superblock");
        assert_eq!(sb.data_bitmap_start, sb.inode_bitmap_start + sb.inode_bitmap_len);
        assert_eq!(sb.inode_table_start, sb.data_bitmap_start + sb.data_bitmap_len);
        assert_eq!(sb.data_start, sb.inode_table_start + sb.inode_table_len);
        assert_eq!(sb.data_start + sb.data_len, sb.total_blocks, "regions tile the whole disk");
    }

    #[test]
    fn root_is_an_empty_directory() {
        let fs = fresh();
        assert_eq!(fs.root_inode(), ROOT_INODE);
        assert_eq!(fs.inode_type(ROOT_INODE), Some(InodeType::Dir));
        assert_eq!(fs.inode_size(ROOT_INODE), 0);
    }

    #[test]
    fn small_file_round_trip() {
        let fs = fresh();
        let id = fs.alloc_inode(InodeType::File).unwrap();
        let data = b"hello, rfs";
        assert_eq!(fs.write_at(id, 0, data), data.len());
        assert_eq!(fs.inode_size(id), data.len());

        let mut buf = [0u8; 32];
        let n = fs.read_at(id, 0, &mut buf);
        assert_eq!(n, data.len());
        assert_eq!(&buf[..n], data);
    }

    #[test]
    fn write_read_across_direct_blocks() {
        let fs = fresh();
        let id = fs.alloc_inode(InodeType::File).unwrap();
        let data = pattern(5000); // > 9 blocks, all direct
        fs.write_at(id, 0, &data);

        let mut buf = vec![0u8; data.len()];
        assert_eq!(fs.read_at(id, 0, &mut buf), data.len());
        assert_eq!(buf, data, "content survives multi-block direct write");
    }

    #[test]
    fn write_read_into_single_indirect() {
        let fs = fresh();
        let id = fs.alloc_inode(InodeType::File).unwrap();
        let data = pattern(20_000); // > 26*512 = 13312, spills into single indirect
        fs.write_at(id, 0, &data);
        assert_ne!(fs.read_disk_inode(id, |di| di.indirect), 0, "single-indirect block used");

        let mut buf = vec![0u8; data.len()];
        assert_eq!(fs.read_at(id, 0, &mut buf), data.len());
        assert_eq!(buf, data);
    }

    #[test]
    fn write_read_into_double_indirect() {
        let fs = fresh();
        let id = fs.alloc_inode(InodeType::File).unwrap();
        let data = pattern(100_000); // > 77 KiB, reaches double indirect
        fs.write_at(id, 0, &data);
        assert_ne!(fs.read_disk_inode(id, |di| di.double_indirect), 0, "double-indirect used");

        let mut buf = vec![0u8; data.len()];
        assert_eq!(fs.read_at(id, 0, &mut buf), data.len());
        assert_eq!(buf, data);
    }

    #[test]
    fn partial_overwrite_leaves_neighbours_intact() {
        let fs = fresh();
        let id = fs.alloc_inode(InodeType::File).unwrap();
        let mut expect = pattern(2000);
        fs.write_at(id, 0, &expect);

        let patch = [0xFFu8; 100];
        fs.write_at(id, 500, &patch);
        expect[500..600].copy_from_slice(&patch);

        let mut buf = vec![0u8; 2000];
        fs.read_at(id, 0, &mut buf);
        assert_eq!(buf, expect, "overwrite touches only its own range");
    }

    #[test]
    fn read_is_clamped_to_size() {
        let fs = fresh();
        let id = fs.alloc_inode(InodeType::File).unwrap();
        fs.write_at(id, 0, &pattern(100));

        let mut buf = [0u8; 200];
        assert_eq!(fs.read_at(id, 0, &mut buf), 100, "read past EOF is clamped");
        assert_eq!(fs.read_at(id, 100, &mut buf), 0, "read at EOF yields nothing");
        assert_eq!(fs.read_at(id, 500, &mut buf), 0, "read beyond EOF yields nothing");
    }

    #[test]
    fn sparse_write_reads_hole_as_zeros() {
        let fs = fresh();
        let id = fs.alloc_inode(InodeType::File).unwrap();
        // Write only at a high offset, leaving a gap at the start.
        fs.write_at(id, 5000, b"tail");
        assert_eq!(fs.inode_size(id), 5004);

        let mut buf = vec![0u8; 5004];
        fs.read_at(id, 0, &mut buf);
        assert!(buf[..5000].iter().all(|&b| b == 0), "the hole reads as zeros");
        assert_eq!(&buf[5000..], b"tail");
    }

    #[test]
    fn truncate_frees_every_block() {
        let fs = fresh();
        let id = fs.alloc_inode(InodeType::File).unwrap();
        fs.write_at(id, 0, &pattern(100_000)); // direct + single + double indirect
        assert!(fs.used_data_blocks() > 0);

        fs.truncate(id);
        assert_eq!(fs.inode_size(id), 0);
        assert_eq!(fs.used_data_blocks(), 0, "truncate returns all data and indirect blocks");
    }

    #[test]
    fn free_inode_reclaims_inode_and_data() {
        let fs = fresh();
        let id = fs.alloc_inode(InodeType::File).unwrap();
        fs.write_at(id, 0, &pattern(30_000));
        fs.free_inode(id);
        assert_eq!(fs.used_data_blocks(), 0, "freeing an inode frees its data");
        // The inode number is available again.
        assert_eq!(fs.alloc_inode(InodeType::File), Some(id), "freed inode is reused");
    }

    #[test]
    fn survives_reopen() {
        let ram = test_ram();
        let (id, data) = {
            let fs = format_on(&ram);
            let id = fs.alloc_inode(InodeType::File).unwrap();
            let data = pattern(40_000);
            fs.write_at(id, 0, &data);
            fs.sync();
            (id, data)
        };
        // Remount over the same disk.
        let fs = mount_on(&ram);
        let mut buf = vec![0u8; data.len()];
        assert_eq!(fs.read_at(id, 0, &mut buf), data.len());
        assert_eq!(buf, data, "file content persists across a remount");
    }
}
