//! The storage contract: fixed-size, block-addressed I/O.
//!
//! One trait, defined once, that every backend implements — the kernel's
//! virtio-blk driver, or a [`RamDisk`](crate::RamDisk) in tests and tooling —
//! and that every consumer (the filesystem, tooling) is written against.

/// Size of one logical block, in bytes. Equal to a single 512-byte disk /
/// virtio sector, so a logical block maps 1:1 onto a hardware sector — no
/// read-modify-write games at the driver.
pub const BLOCK_SIZE: usize = 512;

/// Random-access, block-addressed storage.
///
/// Everything above this trait reads and writes whole [`BLOCK_SIZE`]-byte
/// blocks through it, so the same consumer runs over a virtio-blk device in the
/// kernel or a [`RamDisk`](crate::RamDisk) in tests. `Send + Sync` so one device
/// can be shared behind an `Arc` across harts.
///
/// # Contract
/// For both methods `buf.len()` must equal [`BLOCK_SIZE`]; implementations may
/// panic otherwise. I/O is treated as infallible: a block device that fails
/// under a mounted filesystem is fatal, so implementations panic rather than
/// thread a `Result` through every caller.
pub trait BlockDevice: Send + Sync {
    /// Read block `block_id` into `buf` (`buf.len() == BLOCK_SIZE`).
    fn read_block(&self, block_id: usize, buf: &mut [u8]);

    /// Write `buf` (`buf.len() == BLOCK_SIZE`) to block `block_id`.
    fn write_block(&self, block_id: usize, buf: &[u8]);
}
