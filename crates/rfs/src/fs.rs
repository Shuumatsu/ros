//! Filesystem mounting, allocation, and inode block mapping.
//!
//! ## Concurrency
//! Individual blocks are locked, but compound filesystem operations are not
//! atomic and require external serialization.

use alloc::sync::Arc;

use bytemuck::Zeroable;

use crate::bitmap::{BITS_PER_BLOCK, Bitmap};
use crate::cache::BlockCacheManager;
use crate::layout::{
    DIRECT_COUNT, DiskInode, FS_MAGIC, INODES_PER_BLOCK, InodeType, MAX_FILE_SIZE,
    POINTERS_PER_BLOCK, ROOT_INODE, SuperBlock,
};
use blockdev::BLOCK_SIZE;

enum BlockSlot {
    Direct(usize),
    SingleIndirect(usize),
    DoubleIndirect { l1: usize, l2: usize },
}

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
    /// Formats and mounts a filesystem with inode 0 as its root directory.
    pub fn format(cache: Arc<BlockCacheManager>, total_blocks: usize, ninodes: usize) -> Fs {
        let inode_bitmap_len = ninodes.div_ceil(BITS_PER_BLOCK);
        let inode_table_len = ninodes.div_ceil(INODES_PER_BLOCK);

        // Each data-bitmap block tracks BITS_PER_BLOCK blocks and occupies one.
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

        // Metadata is zeroed eagerly; data blocks are zeroed on allocation.
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

        let root = fs.alloc_inode(InodeType::Dir).expect("rfs: cannot allocate root inode");
        assert_eq!(root, ROOT_INODE, "rfs: root must be inode 0");
        fs.sync();
        fs
    }

    /// Mounts a formatted device, panicking if its magic is invalid.
    pub fn mount(cache: Arc<BlockCacheManager>) -> Fs {
        let sb = cache.get(0).lock().read(0, |sb: &SuperBlock| *sb);
        assert_eq!(sb.magic, FS_MAGIC, "rfs: bad superblock magic {:#x}", sb.magic);
        let inode_bitmap =
            Bitmap::new(sb.inode_bitmap_start as usize, sb.inode_bitmap_len as usize);
        let data_bitmap = Bitmap::new(sb.data_bitmap_start as usize, sb.data_bitmap_len as usize);
        Fs { cache, sb, inode_bitmap, data_bitmap }
    }

    pub fn root_inode(&self) -> u32 { self.sb.root_inode }

    pub fn superblock(&self) -> &SuperBlock { &self.sb }

    /// Flushes dirty cached blocks to the device.
    pub fn sync(&self) { self.cache.sync_all(); }

    pub fn inode_size(&self, id: u32) -> usize { self.read_disk_inode(id, |di| di.size as usize) }

    pub fn inode_type(&self, id: u32) -> Option<InodeType> {
        self.read_disk_inode(id, |di| InodeType::from_raw(di.type_))
    }

    /// Allocates an empty inode with one link, or returns `None` if full.
    pub fn alloc_inode(&self, kind: InodeType) -> Option<u32> {
        let bit = self.inode_bitmap.alloc(&self.cache)?;
        // The final bitmap block may contain bits beyond the inode table.
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

    /// Releases an inode and all of its data.
    pub fn free_inode(&self, id: u32) {
        self.set_len(id, 0);
        self.modify_disk_inode(id, |di| *di = DiskInode::zeroed());
        self.inode_bitmap.dealloc(&self.cache, id as usize);
    }

    pub fn read_disk_inode<V>(&self, id: u32, f: impl FnOnce(&DiskInode) -> V) -> V {
        let (block, offset) = self.inode_pos(id);
        self.cache.get(block).lock().read(offset, f)
    }

    pub fn modify_disk_inode<V>(&self, id: u32, f: impl FnOnce(&mut DiskInode) -> V) -> V {
        let (block, offset) = self.inode_pos(id);
        self.cache.get(block).lock().modify(offset, f)
    }

    fn inode_pos(&self, id: u32) -> (usize, usize) {
        let block = self.sb.inode_table_start as usize + id as usize / INODES_PER_BLOCK;
        let offset = (id as usize % INODES_PER_BLOCK) * core::mem::size_of::<DiskInode>();
        (block, offset)
    }

    /// Allocates a zeroed block; zero is the unallocated pointer representation.
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

    fn free_data_block(&self, block: usize) {
        let bit = block - self.sb.data_start as usize;
        self.data_bitmap.dealloc(&self.cache, bit);
    }

    /// Reads up to `buf.len()` bytes, returning 0 at or beyond EOF.
    /// Unallocated ranges read as zeros.
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

    /// Writes and grows as needed, returning `buf.len()` on success.
    ///
    /// The size limit is checked before writing. Allocation failure may panic
    /// after partially modifying blocks; those modifications are not rolled back.
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

    /// Resizes an inode without reallocating on growth.
    ///
    /// Shrinking frees blocks beyond EOF and zeros the truncated tail of a
    /// retained block. Grown ranges remain holes and read as zeros.
    pub fn set_len(&self, id: u32, new_len: usize) {
        assert!(new_len <= MAX_FILE_SIZE, "rfs: length {new_len} exceeds {MAX_FILE_SIZE}");
        if new_len < self.inode_size(id) {
            self.free_from(id, new_len);
        }
        self.modify_disk_inode(id, |di| di.size = new_len as u32);
    }

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

    fn ensure_inode_pointer(&self, id: u32, field: impl Fn(&mut DiskInode) -> &mut u32) -> usize {
        let existing = self.modify_disk_inode(id, |di| *field(di));
        if existing != 0 {
            return existing as usize;
        }
        let block = self.alloc_data_block().expect("rfs: out of data blocks");
        self.modify_disk_inode(id, |di| *field(di) = block as u32);
        block
    }

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

    fn read_pointer(&self, indirect: usize, idx: usize) -> Option<usize> {
        let offset = idx * core::mem::size_of::<u32>();
        let p = self.cache.get(indirect).lock().read(offset, |p: &u32| *p);
        (p != 0).then_some(p as usize)
    }

    fn inode_pointer(&self, id: u32, field: impl Fn(&DiskInode) -> u32) -> Option<usize> {
        let block = self.read_disk_inode(id, |di| field(di));
        (block != 0).then_some(block as usize)
    }

    /// Frees blocks beyond `new_len` and zeroes bytes after EOF in the retained block.
    fn free_from(&self, id: u32, new_len: usize) {
        let keep = new_len.div_ceil(BLOCK_SIZE);
        let tail = new_len % BLOCK_SIZE;
        if tail != 0 {
            let di = self.read_disk_inode(id, |di| *di);
            if let Some(block) = self.block_map(&di, keep - 1) {
                self.cache.get(block).lock().modify(0, |blk: &mut [u8; BLOCK_SIZE]| {
                    blk[tail..].fill(0);
                });
            }
        }
        match locate_block(keep) {
            BlockSlot::Direct(i) => {
                let direct = self.read_disk_inode(id, |di| di.direct);
                for &block in &direct[i..] {
                    if block != 0 {
                        self.free_data_block(block as usize);
                    }
                }
                self.modify_disk_inode(id, |di| di.direct[i..].fill(0));
                self.free_inode_tree(id, |di| &mut di.indirect, Self::free_indirect);
                self.free_inode_tree(id, |di| &mut di.double_indirect, Self::free_double_indirect);
            }
            BlockSlot::SingleIndirect(i) => {
                if i == 0 {
                    self.free_inode_tree(id, |di| &mut di.indirect, Self::free_indirect);
                } else if let Some(block) = self.inode_pointer(id, |di| di.indirect) {
                    self.free_indirect_from(block, i);
                }
                self.free_inode_tree(id, |di| &mut di.double_indirect, Self::free_double_indirect);
            }
            BlockSlot::DoubleIndirect { l1, l2 } => {
                if (l1, l2) == (0, 0) {
                    self.free_inode_tree(
                        id,
                        |di| &mut di.double_indirect,
                        Self::free_double_indirect,
                    );
                } else if let Some(double) = self.inode_pointer(id, |di| di.double_indirect) {
                    if l2 != 0
                        && let Some(mid) = self.read_pointer(double, l1)
                    {
                        self.free_indirect_from(mid, l2);
                    }
                    self.free_double_from(double, if l2 == 0 { l1 } else { l1 + 1 });
                }
            }
        }
    }

    fn free_inode_tree(
        &self,
        id: u32,
        field: impl Fn(&mut DiskInode) -> &mut u32,
        free: impl Fn(&Self, usize),
    ) {
        let block = self.modify_disk_inode(id, |di| core::mem::replace(field(di), 0));
        if block != 0 {
            free(self, block as usize);
        }
    }

    /// Clears released pointers when the index block survives.
    fn free_entries_from(&self, index: usize, from: usize, free: impl Fn(&Self, usize)) {
        let pointers = self.cache.get(index).lock().read(0, |p: &[u32; POINTERS_PER_BLOCK]| *p);
        for &p in &pointers[from..] {
            if p != 0 {
                free(self, p as usize);
            }
        }
        if from != 0 {
            self.cache
                .get(index)
                .lock()
                .modify(0, |p: &mut [u32; POINTERS_PER_BLOCK]| p[from..].fill(0));
        }
    }

    fn free_indirect_from(&self, indirect: usize, from: usize) {
        self.free_entries_from(indirect, from, Self::free_data_block);
    }

    fn free_double_from(&self, double: usize, from: usize) {
        self.free_entries_from(double, from, Self::free_indirect);
    }

    fn free_indirect(&self, block: usize) {
        self.free_entries_from(block, 0, Self::free_data_block);
        self.free_data_block(block);
    }

    fn free_double_indirect(&self, block: usize) {
        self.free_entries_from(block, 0, Self::free_indirect);
        self.free_data_block(block);
    }

    #[cfg(test)]
    fn used_data_blocks(&self) -> usize {
        (0..self.sb.data_len as usize)
            .filter(|&i| self.data_bitmap.is_allocated(&self.cache, i))
            .count()
    }
}

#[cfg(test)]
mod tests {
    use crate::layout::{DIRECT_COUNT, FS_MAGIC, InodeType, POINTERS_PER_BLOCK, ROOT_INODE};
    use crate::test_support::{format_on, fresh, mount_on, pattern, test_ram};
    use alloc::vec;
    use blockdev::BLOCK_SIZE;
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
        let data = pattern(5000);
        fs.write_at(id, 0, &data);

        let mut buf = vec![0u8; data.len()];
        assert_eq!(fs.read_at(id, 0, &mut buf), data.len());
        assert_eq!(buf, data, "content survives multi-block direct write");
    }

    #[test]
    fn write_read_into_single_indirect() {
        let fs = fresh();
        let id = fs.alloc_inode(InodeType::File).unwrap();
        let data = pattern(20_000);
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
        let data = pattern(100_000);
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
        fs.write_at(id, 5000, b"tail");
        assert_eq!(fs.inode_size(id), 5004);

        let mut buf = vec![0u8; 5004];
        fs.read_at(id, 0, &mut buf);
        assert!(buf[..5000].iter().all(|&b| b == 0), "the hole reads as zeros");
        assert_eq!(&buf[5000..], b"tail");
    }

    #[test]
    fn set_len_zero_frees_every_block() {
        let fs = fresh();
        let id = fs.alloc_inode(InodeType::File).unwrap();
        fs.write_at(id, 0, &pattern(100_000));
        assert!(fs.used_data_blocks() > 0);

        fs.set_len(id, 0);
        assert_eq!(fs.inode_size(id), 0);
        assert_eq!(fs.used_data_blocks(), 0, "truncate returns all data and indirect blocks");
    }

    #[test]
    fn shrink_frees_exactly_the_blocks_past_the_end() {
        const D: usize = DIRECT_COUNT * BLOCK_SIZE;
        const S: usize = D + POINTERS_PER_BLOCK * BLOCK_SIZE;
        for &(orig, len) in &[
            (160_000usize, S + POINTERS_PER_BLOCK * BLOCK_SIZE),
            (160_000, 90_000),
            (100_000, S),
            (100_000, 40_000),
            (100_000, D),
            (100_000, 5_000),
            (100_000, 600),
            (100_000, 1),
            (100_000, 0),
        ] {
            let fs = fresh();
            let id = fs.alloc_inode(InodeType::File).unwrap();
            let data = pattern(orig);
            fs.write_at(id, 0, &data);

            let reference = fresh();
            let ref_id = reference.alloc_inode(InodeType::File).unwrap();
            reference.write_at(ref_id, 0, &data[..len]);

            fs.set_len(id, len);
            assert_eq!(fs.inode_size(id), len, "length is exactly what was asked for");
            assert_eq!(
                fs.used_data_blocks(),
                reference.used_data_blocks(),
                "{orig} shrunk to {len} must hold the same blocks as writing {len} bytes"
            );

            let mut buf = vec![0u8; len + 16];
            assert_eq!(fs.read_at(id, 0, &mut buf), len, "read clamps to the new length");
            assert_eq!(buf[..len], data[..len], "surviving bytes are untouched");
        }
    }

    #[test]
    fn shrink_does_not_leave_dangling_pointers() {
        let fs = fresh();
        let victim = fs.alloc_inode(InodeType::File).unwrap();
        let big = pattern(100_000);
        fs.write_at(victim, 0, &big);
        fs.set_len(victim, 40_000);

        let other = fs.alloc_inode(InodeType::File).unwrap();
        let theirs = pattern(60_000).iter().map(|b| !b).collect::<alloc::vec::Vec<u8>>();
        fs.write_at(other, 0, &theirs);

        fs.write_at(victim, 40_000, &big[40_000..]);

        let mut buf = vec![0u8; theirs.len()];
        fs.read_at(other, 0, &mut buf);
        assert_eq!(buf, theirs, "regrowing must not scribble on another file's blocks");
        let mut buf = vec![0u8; big.len()];
        fs.read_at(victim, 0, &mut buf);
        assert_eq!(buf, big, "and the regrown file is itself intact");
    }

    #[test]
    fn shrink_then_grow_reads_zeros_not_old_bytes() {
        let fs = fresh();
        let id = fs.alloc_inode(InodeType::File).unwrap();
        fs.write_at(id, 0, &[0xAA; 400]);

        fs.set_len(id, 100);
        fs.set_len(id, 400);
        let mut buf = [0xFFu8; 400];
        assert_eq!(fs.read_at(id, 0, &mut buf), 400);
        assert!(buf[..100].iter().all(|&b| b == 0xAA), "kept bytes survive");
        assert!(buf[100..].iter().all(|&b| b == 0), "dropped bytes do not come back");
    }

    #[test]
    fn grow_by_set_len_is_a_hole() {
        let fs = fresh();
        let id = fs.alloc_inode(InodeType::File).unwrap();
        fs.set_len(id, 4096);
        assert_eq!(fs.inode_size(id), 4096);
        assert_eq!(fs.used_data_blocks(), 0, "growing allocates nothing until written");

        let mut buf = [0xFFu8; 4096];
        assert_eq!(fs.read_at(id, 0, &mut buf), 4096);
        assert!(buf.iter().all(|&b| b == 0), "the new range reads as zeros");
    }

    #[test]
    fn free_inode_reclaims_inode_and_data() {
        let fs = fresh();
        let id = fs.alloc_inode(InodeType::File).unwrap();
        fs.write_at(id, 0, &pattern(30_000));
        fs.free_inode(id);
        assert_eq!(fs.used_data_blocks(), 0, "freeing an inode frees its data");
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
        let fs = mount_on(&ram);
        let mut buf = vec![0u8; data.len()];
        assert_eq!(fs.read_at(id, 0, &mut buf), data.len());
        assert_eq!(buf, data, "file content persists across a remount");
    }
}
