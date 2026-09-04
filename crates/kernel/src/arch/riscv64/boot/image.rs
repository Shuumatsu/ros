//! The RISC-V Image header, and `_start`.
//!
//! This 64-byte structure must match Linux's `riscv_image_header` ABI exactly.

const VERSION_MAJOR: u32 = 0;
const VERSION_MINOR: u32 = 2;
const VERSION: u32 = (VERSION_MAJOR << 16) | VERSION_MINOR;

const MAGIC2: u32 = u32::from_le_bytes(*b"RSC\x05");

boot_fn!(
    /// Image offset zero and the ELF entry point.
    #[unsafe(no_mangle)]
    pub(super) fn _start in header {
        "j {boot}",                     // 0x00 code0:       the only instruction here
        ".4byte 0",                     // 0x04 code1
        ".8byte _text_offset",          // 0x08 text_offset: load offset from the RAM base
        ".8byte _image_size",           // 0x10 image_size:  bytes the loader must reserve
        ".8byte 0",                     // 0x18 flags:       bit 0 clear, i.e. little endian
        ".4byte {version}",             // 0x20 version
        ".4byte 0",                     // 0x24 res1
        ".8byte 0",                     // 0x28 res2
        ".8byte 0",                     // 0x30 magic:       deprecated for this version
        ".4byte {magic2}",              // 0x38 magic2
        ".4byte 0",                     // 0x3c res3:        PE/COFF offset, unused
        // `kernel.ld` asserts this offset is 0x40 and `_start` is the image's first byte.
        ".globl _image_header_end",
        "_image_header_end:",
    }
        boot = sym super::entry::primary_entry,
        version = const VERSION,
        magic2 = const MAGIC2,
);
