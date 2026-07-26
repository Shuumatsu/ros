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

/// Longest file name, in bytes — the full capacity of [`DirEntry::name`], since
/// the length is stored explicitly and no byte is spent on a terminator.
pub const NAME_MAX: usize = 27;

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
/// The name is stored as an explicit length plus a fixed byte array, *not*
/// NUL-terminated: the length is then O(1) to read, no byte of the 27 is spent
/// on a terminator, and a name may contain any byte. `name_len` comes off disk
/// unvalidated, so [`DirEntry::name`] clamps it — a corrupt entry must not be
/// able to slice out of bounds.
///
/// Every slot in `[0, size/32)` of a directory is live: removal compacts by
/// moving the last entry down, so there are no free slots to skip and no
/// sentinel value to reserve (see [`Fs::remove`](crate::Fs::remove)).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
pub struct DirEntry {
    pub inode: u32,
    /// Bytes of `name` in use; at most [`NAME_MAX`].
    pub name_len: u8,
    pub name: [u8; NAME_MAX],
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
    /// Build an entry for `inode` named `name`, truncated to [`NAME_MAX`] bytes.
    pub fn new(inode: u32, name: &str) -> Self {
        let bytes = name.as_bytes();
        let n = if bytes.len() > NAME_MAX { NAME_MAX } else { bytes.len() };
        let mut entry = Self { inode, name_len: n as u8, name: [0u8; NAME_MAX] };
        entry.name[..n].copy_from_slice(&bytes[..n]);
        entry
    }

    /// The name as a string slice. A `name_len` past the field (only reachable
    /// from a corrupt image) is clamped; invalid UTF-8 reads back as empty.
    pub fn name(&self) -> &str {
        let end = (self.name_len as usize).min(NAME_MAX);
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
        assert_eq!(entry.name_len as usize, "hello.txt".len(), "length stored explicitly");
        assert_eq!(entry.name(), "hello.txt");
    }

    #[test]
    fn dir_entry_name_fills_the_field_and_truncates() {
        // Exactly NAME_MAX bytes: no terminator to spare, all 27 usable.
        let full = "a".repeat(NAME_MAX);
        assert_eq!(DirEntry::new(1, &full).name(), full, "a full-width name survives");

        // 36 ASCII bytes, longer than NAME_MAX.
        let long = "abcdefghijklmnopqrstuvwxyz0123456789";
        let entry = DirEntry::new(1, long);
        assert_eq!(entry.name_len as usize, NAME_MAX, "name truncated to NAME_MAX");
        assert_eq!(entry.name(), &long[..NAME_MAX]);
    }

    #[test]
    fn dir_entry_clamps_a_corrupt_length() {
        // A garbage name_len off a corrupt disk must not slice out of bounds.
        let mut entry = DirEntry::new(1, "ok");
        entry.name_len = u8::MAX;
        assert_eq!(entry.name().len(), NAME_MAX, "clamped to the field, no panic");
    }
}
