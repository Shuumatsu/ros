//! The RISC-V Image header, and `_start`.
//!
//! Byte-for-byte Linux's `struct riscv_image_header`: 64 bytes opening with the branch the
//! loader enters at, then the fields it reads to decide where and how much to load. An ABI
//! somebody else owns, so each field carries its offset below and the reserved ones are
//! zero.
//!
//! This is what makes the flat binary `scripts/run.sh` boots bootable.

/// Header revision this image declares, `major << 16 | minor`. A loader compares the two
/// halves separately.
const VERSION_MAJOR: u32 = 0;
const VERSION_MINOR: u32 = 2;
const VERSION: u32 = (VERSION_MAJOR << 16) | VERSION_MINOR;

/// `magic2`, the field a loader matches to recognise an Image.
const MAGIC2: u32 = u32::from_le_bytes(*b"RSC\x05");

/// Offset zero of the image, and the ELF entry point: a branch past the header, then the
/// 60 bytes the loader reads.
///
/// `_text_offset` and `_image_size` come from `kernel.ld` by name. Both are small absolute
/// linker symbols, out of reach of a Rust `extern static` (see [`crate::memory::layout`]).
#[unsafe(naked)]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.init.header")]
pub(super) unsafe extern "custom" fn _start() {
    boot_asm!({
        "j {boot}",                     // 0x00 code0:       the only instruction here
        ".4byte 0",                     // 0x04 code1
        ".8byte _text_offset",          // 0x08 text_offset: load offset from the RAM base
        ".8byte _image_size",           // 0x10 image_size:  bytes the loader must reserve
        ".8byte 0",                     // 0x18 flags:       bit 0 clear, i.e. little endian
        ".4byte {version}",             // 0x20 version
        ".4byte 0",                     // 0x24 res1
        ".8byte 0",                     // 0x28 res2
        ".8byte 0",                     // 0x30 magic:       "RISCV\0\0\0", deprecated at
                                        //                   the version declared above
        ".4byte {magic2}",              // 0x38 magic2
        ".4byte 0",                     // 0x3c res3:        PE/COFF offset, unused
        // 0x40. `kernel.ld` asserts the header is exactly this long, and that `_start` is
        // the image's first byte.
        ".globl _image_header_end",
        "_image_header_end:",
    }
        boot = sym super::entry::primary_entry,
        version = const VERSION,
        magic2 = const MAGIC2,
    )
}
