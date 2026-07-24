//! An in-RAM block device: the whole disk held as one byte vector. Pure
//! `alloc`, no `std` — used by unit tests and the `mkfs` host tool, and
//! available to the kernel too (a RAM-backed filesystem, or an in-memory image
//! during bring-up before a real block driver exists).

use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

use spin::Mutex;

use crate::{BLOCK_SIZE, BlockDevice};

/// A whole disk held in memory as one contiguous byte vector.
pub struct RamDisk {
    data: Mutex<Vec<u8>>,
    writes: AtomicUsize,
}

impl RamDisk {
    /// A zeroed disk of `blocks` blocks.
    pub fn new(blocks: usize) -> Self {
        Self { data: Mutex::new(vec![0u8; blocks * BLOCK_SIZE]), writes: AtomicUsize::new(0) }
    }

    /// Adopt an existing image; its length must be a whole number of blocks.
    pub fn from_image(image: Vec<u8>) -> Self {
        assert!(image.len().is_multiple_of(BLOCK_SIZE), "image is not a whole number of blocks");
        Self { data: Mutex::new(image), writes: AtomicUsize::new(0) }
    }

    /// Number of blocks.
    pub fn blocks(&self) -> usize { self.data.lock().len() / BLOCK_SIZE }

    /// A copy of the whole disk, e.g. to write out to a file.
    pub fn snapshot(&self) -> Vec<u8> { self.data.lock().clone() }

    /// How many `write_block` calls have hit the device — lets tests prove a
    /// cache does not write back clean blocks.
    pub fn writes(&self) -> usize { self.writes.load(Ordering::Relaxed) }
}

impl BlockDevice for RamDisk {
    fn read_block(&self, block_id: usize, buf: &mut [u8]) {
        assert_eq!(buf.len(), BLOCK_SIZE, "read buffer must be exactly one block");
        let data = self.data.lock();
        let start = block_id * BLOCK_SIZE;
        buf.copy_from_slice(&data[start..start + BLOCK_SIZE]);
    }

    fn write_block(&self, block_id: usize, buf: &[u8]) {
        assert_eq!(buf.len(), BLOCK_SIZE, "write buffer must be exactly one block");
        self.writes.fetch_add(1, Ordering::Relaxed);
        let mut data = self.data.lock();
        let start = block_id * BLOCK_SIZE;
        data[start..start + BLOCK_SIZE].copy_from_slice(buf);
    }
}

#[cfg(test)]
mod tests {
    use super::RamDisk;
    use crate::{BLOCK_SIZE, BlockDevice};
    use alloc::vec;
    use pretty_assertions::assert_eq;

    #[test]
    fn write_then_read_roundtrips() {
        let disk = RamDisk::new(4);
        let mut w = [0u8; BLOCK_SIZE];
        w[0] = 0xAB;
        w[BLOCK_SIZE - 1] = 0xCD;
        disk.write_block(2, &w);

        let mut r = [0u8; BLOCK_SIZE];
        disk.read_block(2, &mut r);
        assert_eq!(r, w, "a written block reads back verbatim");
        assert_eq!(disk.writes(), 1, "one write recorded");
    }

    #[test]
    fn snapshot_and_from_image_roundtrip() {
        let disk = RamDisk::new(2);
        let mut b = [0u8; BLOCK_SIZE];
        b[..3].copy_from_slice(b"hey");
        disk.write_block(1, &b);

        let image = disk.snapshot();
        assert_eq!(image.len(), 2 * BLOCK_SIZE);

        let reloaded = RamDisk::from_image(image);
        assert_eq!(reloaded.blocks(), 2);
        let mut r = [0u8; BLOCK_SIZE];
        reloaded.read_block(1, &mut r);
        assert_eq!(&r[..3], b"hey", "image survives snapshot + reload");
    }

    #[test]
    #[should_panic(expected = "whole number of blocks")]
    fn from_image_rejects_partial_block() { RamDisk::from_image(vec![0u8; BLOCK_SIZE + 1]); }
}
