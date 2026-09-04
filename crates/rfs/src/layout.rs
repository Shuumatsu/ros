//! Persistent rfs layout.
//!
//! ```text
//! ┌──────────┬──────────────┬─────────────┬─────────────┬─────────────┐
//! │ super(0) │ inode bitmap │ data bitmap │ inode table │ data blocks │
//! └──────────┴──────────────┴─────────────┴─────────────┴─────────────┘
//! ```
//!
//! [`SuperBlock`] records every region in blocks. On-disk integers are
//! fixed-width little-endian values, and `#[repr(C)]` structures have
//! compile-time-checked sizes. Inode kinds remain raw until validated.

use bytemuck::{Pod, Zeroable};

use blockdev::BLOCK_SIZE;

/// Superblock magic, written as the hexadecimal mnemonic `RFS1`.
pub const FS_MAGIC: u32 = 0x5246_5331;

pub const ROOT_INODE: u32 = 0;

pub const INODES_PER_BLOCK: usize = BLOCK_SIZE / core::mem::size_of::<DiskInode>();

pub const POINTERS_PER_BLOCK: usize = BLOCK_SIZE / core::mem::size_of::<u32>();

pub const DIRECT_COUNT: usize = 26;

/// Maximum file-name length in bytes; names are not NUL-terminated.
pub const NAME_MAX: usize = 27;

/// Maximum blocks addressable by the direct, single-, and double-indirect pointers.
pub const MAX_FILE_BLOCKS: usize =
    DIRECT_COUNT + POINTERS_PER_BLOCK + POINTERS_PER_BLOCK * POINTERS_PER_BLOCK;

/// Maximum file length in bytes.
pub const MAX_FILE_SIZE: usize = MAX_FILE_BLOCKS * BLOCK_SIZE;

/// The filesystem superblock, stored in block 0.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
pub struct SuperBlock {
    pub magic: u32,
    pub block_size: u32,
    pub total_blocks: u32,
    pub ninodes: u32,
    pub inode_bitmap_start: u32,
    pub inode_bitmap_len: u32,
    pub data_bitmap_start: u32,
    pub data_bitmap_len: u32,
    pub inode_table_start: u32,
    pub inode_table_len: u32,
    pub data_start: u32,
    pub data_len: u32,
    pub root_inode: u32,
}

/// A 128-byte on-disk inode.
///
/// Blocks are addressed directly, then through single- and double-indirect
/// pointers. Zero denotes an unallocated pointer.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
pub struct DiskInode {
    /// Raw [`InodeType`]; use [`InodeType::from_raw`] to interpret.
    pub type_: u32,
    pub size: u32,
    pub nlink: u32,
    pub _reserved: u32,
    pub direct: [u32; DIRECT_COUNT],
    pub indirect: u32,
    pub double_indirect: u32,
}

/// A 32-byte directory entry.
///
/// Names use an explicit length and are not NUL-terminated. Every slot below a
/// directory's size is live; removal compacts entries and does not preserve order.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
pub struct DirEntry {
    pub inode: u32,
    pub name_len: u8,
    pub name: [u8; NAME_MAX],
}

/// Validated interpretation of [`DiskInode::type_`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InodeType {
    Free = 0,
    File = 1,
    Dir = 2,
}

impl InodeType {
    pub const fn from_raw(raw: u32) -> Option<Self> {
        match raw {
            0 => Some(Self::Free),
            1 => Some(Self::File),
            2 => Some(Self::Dir),
            _ => None,
        }
    }

    pub const fn as_raw(self) -> u32 { self as u32 }
}

impl DirEntry {
    /// Builds an entry, truncating `name` to [`NAME_MAX`] bytes.
    pub fn new(inode: u32, name: &str) -> Self {
        let bytes = name.as_bytes();
        let n = if bytes.len() > NAME_MAX { NAME_MAX } else { bytes.len() };
        let mut entry = Self { inode, name_len: n as u8, name: [0u8; NAME_MAX] };
        entry.name[..n].copy_from_slice(&bytes[..n]);
        entry
    }

    /// Returns the name, clamping corrupt lengths and mapping invalid UTF-8 to empty.
    pub fn name(&self) -> &str {
        let end = (self.name_len as usize).min(NAME_MAX);
        core::str::from_utf8(&self.name[..end]).unwrap_or("")
    }
}

// On-disk ABI invariants.
const_assert_eq!(core::mem::size_of::<SuperBlock>(), 52);
const_assert_eq!(core::mem::size_of::<DiskInode>(), 128);
const_assert_eq!(core::mem::size_of::<DirEntry>(), 32);
const_assert_eq!(BLOCK_SIZE % core::mem::size_of::<DiskInode>(), 0);
const_assert_eq!(BLOCK_SIZE % core::mem::size_of::<DirEntry>(), 0);
const_assert_eq!(INODES_PER_BLOCK, 4);
const_assert_eq!(POINTERS_PER_BLOCK, 128);
const_assert!(core::mem::size_of::<SuperBlock>() <= BLOCK_SIZE);

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn on_disk_sizes_are_fixed() {
        assert_eq!(core::mem::size_of::<SuperBlock>(), 52, "SuperBlock");
        assert_eq!(core::mem::size_of::<DiskInode>(), 128, "DiskInode");
        assert_eq!(core::mem::size_of::<DirEntry>(), 32, "DirEntry");
        assert_eq!(INODES_PER_BLOCK, 4, "inodes per block");
        assert_eq!(POINTERS_PER_BLOCK, 128, "pointers per indirect block");
    }

    #[test]
    fn max_file_size_spans_double_indirect() {
        assert_eq!(MAX_FILE_BLOCKS, 26 + 128 + 128 * 128, "direct + single + double indirect");
        assert_eq!(MAX_FILE_SIZE, MAX_FILE_BLOCKS * BLOCK_SIZE);
    }

    #[test]
    fn inode_type_roundtrips_and_rejects_garbage() {
        for t in [InodeType::Free, InodeType::File, InodeType::Dir] {
            assert_eq!(InodeType::from_raw(t.as_raw()), Some(t), "roundtrip {t:?}");
        }
        assert_eq!(InodeType::from_raw(3), None, "unknown discriminant rejected");
        assert_eq!(InodeType::from_raw(u32::MAX), None, "garbage rejected");
    }

    #[test]
    fn dir_entry_name_roundtrip() {
        let entry = DirEntry::new(7, "hello.txt");
        assert_eq!(entry.inode, 7);
        assert_eq!(entry.name_len as usize, "hello.txt".len(), "length stored explicitly");
        assert_eq!(entry.name(), "hello.txt");
    }

    #[test]
    fn dir_entry_name_fills_the_field_and_truncates() {
        let full = "a".repeat(NAME_MAX);
        assert_eq!(DirEntry::new(1, &full).name(), full, "a full-width name survives");

        let long = "abcdefghijklmnopqrstuvwxyz0123456789";
        let entry = DirEntry::new(1, long);
        assert_eq!(entry.name_len as usize, NAME_MAX, "name truncated to NAME_MAX");
        assert_eq!(entry.name(), &long[..NAME_MAX]);
    }

    #[test]
    fn dir_entry_clamps_a_corrupt_length() {
        let mut entry = DirEntry::new(1, "ok");
        entry.name_len = u8::MAX;
        assert_eq!(entry.name().len(), NAME_MAX, "clamped to the field, no panic");
    }
}
