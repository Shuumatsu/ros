//! Fixed-size, block-addressed storage.

/// Size of one logical block in bytes.
pub const BLOCK_SIZE: usize = 512;

/// Random-access, block-addressed storage.
///
/// Buffers must be exactly [`BLOCK_SIZE`] bytes; implementations may panic
/// otherwise. I/O failure is fatal and is reported by panicking.
pub trait BlockDevice: Send + Sync {
    fn read_block(&self, block_id: usize, buf: &mut [u8]);

    fn write_block(&self, block_id: usize, buf: &[u8]);
}
