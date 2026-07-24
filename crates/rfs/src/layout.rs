//! On-disk data structures and the fixed layout of an rfs image.
//!
//! An rfs image is a sequence of [`BLOCK_SIZE`]-byte blocks:
//!
//! ```text
//! ┌──────────┬──────────────┬─────────────┬─────────────┬─────────────┐
//! │ super(0) │ inode bitmap │ data bitmap │ inode table │ data blocks │
//! └──────────┴──────────────┴─────────────┴─────────────┴─────────────┘
//! ```
//!
//! The [`SuperBlock`] in block 0 records the start and length of every region,
//! so the layout is self-describing: nothing here hard-codes where a region
//! sits for a given image size — [`crate`]'s formatter computes it and the
//! reader trusts the superblock.
//!
//! On-disk integers are fixed-width little-endian `u32` (block and inode
//! indices), never host `usize`, so an image is byte-identical no matter who
//! wrote it. Every structure is `#[repr(C)]` with no padding, so a block buffer
//! can be reinterpreted field-for-field. `type_` is a raw `u32` rather than an
//! enum on purpose: a corrupt block must never be reinterpreted into an invalid
//! enum discriminant (undefined behaviour) — [`InodeType`] does the checked
//! conversion.

use bytemuck::{Pod, Zeroable};

use blockdev::BLOCK_SIZE;

/// Superblock magic: ASCII `"RFS1"`, read big-endian in a hex dump.
pub const FS_MAGIC: u32 = 0x5246_5331;

/// Root directory inode number. Fixed by convention.
pub const ROOT_INODE: u32 = 0;

/// [`DiskInode`]s per block.
pub const INODES_PER_BLOCK: usize = BLOCK_SIZE / core::mem::size_of::<DiskInode>();

/// `u32` block pointers per (indirect) block.
pub const POINTERS_PER_BLOCK: usize = BLOCK_SIZE / core::mem::size_of::<u32>();

/// Direct block pointers stored inline in an inode.
pub const DIRECT_COUNT: usize = 26;

/// Capacity of the [`DirEntry::name`] field, in bytes.
pub const NAME_CAP: usize = 28;

/// Longest usable file name, in bytes. One byte is reserved so a name is always
/// NUL-terminated within [`NAME_CAP`].
pub const NAME_MAX: usize = NAME_CAP - 1;

/// Largest file addressable by one inode, in blocks: the direct pointers, plus
/// one single-indirect block, plus one double-indirect block.
pub const MAX_FILE_BLOCKS: usize =
    DIRECT_COUNT + POINTERS_PER_BLOCK + POINTERS_PER_BLOCK * POINTERS_PER_BLOCK;

/// Largest file addressable by one inode, in bytes (≈ 8 MiB).
pub const MAX_FILE_SIZE: usize = MAX_FILE_BLOCKS * BLOCK_SIZE;

/// The filesystem superblock, stored in block 0.
///
/// Each region is described by an `(start, len)` pair measured in blocks, which
/// is what makes the image self-describing and resizable without code changes.
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

/// An on-disk inode: 128 bytes, [`INODES_PER_BLOCK`] to a block.
///
/// Data blocks are addressed by [`DIRECT_COUNT`] direct pointers, then one
/// single-indirect pointer ([`POINTERS_PER_BLOCK`] blocks) and one
/// double-indirect pointer ([`POINTERS_PER_BLOCK`]² blocks). A pointer of `0`
/// means "no block": block 0 is the superblock and can never be a file's data.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
pub struct DiskInode {
    /// Raw [`InodeType`]; use [`InodeType::from_raw`] to interpret.
    pub type_: u32,
    /// File length in bytes (for a directory, the byte length of its entries).
    pub size: u32,
    /// Hard-link count.
    pub nlink: u32,
    /// Reserved; keeps the struct an exact 128 bytes and leaves room to grow.
    pub _reserved: u32,
    pub direct: [u32; DIRECT_COUNT],
    pub indirect: u32,
    pub double_indirect: u32,
}

/// A directory entry: 32 bytes, 16 to a block.
///
/// `name` is NUL-padded; the usable length is at most [`NAME_MAX`]. An entry
/// with `inode == 0` in a slot past the first is treated as free (inode 0 is
/// the root, which never appears as a child).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
pub struct DirEntry {
    pub inode: u32,
    pub name: [u8; NAME_CAP],
}

/// The kind of object an inode represents. On disk this is the raw `u32`
/// [`DiskInode::type_`]; this enum is the checked, in-memory view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InodeType {
    Free = 0,
    File = 1,
    Dir = 2,
}

impl InodeType {
    /// Interpret a raw on-disk `type_`, or `None` if it is not a known kind.
    pub const fn from_raw(raw: u32) -> Option<Self> {
        match raw {
            0 => Some(Self::Free),
            1 => Some(Self::File),
            2 => Some(Self::Dir),
            _ => None,
        }
    }

    /// The raw value stored in [`DiskInode::type_`].
    pub const fn as_raw(self) -> u32 { self as u32 }
}

impl DirEntry {
    /// An all-zero entry (inode 0, empty name), used to fill free slots.
    pub const fn empty() -> Self { Self { inode: 0, name: [0u8; NAME_CAP] } }

    /// Build an entry for `inode` named `name`. The name is truncated to
    /// [`NAME_MAX`] bytes and always left NUL-terminated.
    pub fn new(inode: u32, name: &str) -> Self {
        let mut entry = Self { inode, name: [0u8; NAME_CAP] };
        let bytes = name.as_bytes();
        let n = if bytes.len() > NAME_MAX { NAME_MAX } else { bytes.len() };
        entry.name[..n].copy_from_slice(&bytes[..n]);
        entry
    }

    /// The name as a string slice: bytes up to the first NUL, as UTF-8. Invalid
    /// UTF-8 (e.g. a name truncated mid-character) reads back as empty.
    pub fn name(&self) -> &str {
        let end = self.name.iter().position(|&b| b == 0).unwrap_or(NAME_CAP);
        core::str::from_utf8(&self.name[..end]).unwrap_or("")
    }
}

// --- Compile-time layout guarantees ------------------------------------------
// These are the load-bearing invariants of the on-disk format. If any changes,
// every existing image becomes unreadable, so pin them at compile time rather
// than trust a comment.
const_assert_eq!(core::mem::size_of::<SuperBlock>(), 52);
const_assert_eq!(core::mem::size_of::<DiskInode>(), 128);
const_assert_eq!(core::mem::size_of::<DirEntry>(), 32);
// Records must tile their blocks exactly — none may straddle a block boundary.
const_assert_eq!(BLOCK_SIZE % core::mem::size_of::<DiskInode>(), 0);
const_assert_eq!(BLOCK_SIZE % core::mem::size_of::<DirEntry>(), 0);
const_assert_eq!(INODES_PER_BLOCK, 4);
const_assert_eq!(POINTERS_PER_BLOCK, 128);
// The superblock must fit in its block.
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
        assert_eq!(entry.name(), "hello.txt");

        let empty = DirEntry::empty();
        assert_eq!(empty.inode, 0);
        assert_eq!(empty.name(), "");
    }

    #[test]
    fn dir_entry_name_truncated_and_terminated() {
        // 36 ASCII bytes, longer than NAME_MAX (27).
        let long = "abcdefghijklmnopqrstuvwxyz0123456789";
        let entry = DirEntry::new(1, long);
        assert_eq!(entry.name().len(), NAME_MAX, "name truncated to NAME_MAX");
        assert_eq!(entry.name.last(), Some(&0u8), "field stays NUL-terminated");
        assert_eq!(entry.name(), &long[..NAME_MAX]);
    }
}
