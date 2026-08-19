//! An executable, as this kernel loads it.
//!
//! ELF, because that is what the toolchain already emits and what carries the two facts a flat
//! image cannot: where each piece belongs, and what rights it wants. Reading the format is the
//! `elf` crate's; what this module owns is which files this kernel agrees to run, and how their
//! program headers become the segments [`crate::memory::user_table`] maps.

use alloc::vec::Vec;
use core::fmt;

use elf::ElfBytes;
use elf::abi::{EM_RISCV, ET_EXEC, PF_R, PF_W, PF_X, PT_LOAD};
use elf::endian::LittleEndian;
use mmu::{PteFlags, VirtualAddr};

use crate::memory::user_table::Segment;

/// A file this kernel is willing to run: where to start, and what to map.
pub struct Image<'a> {
    pub entry: VirtualAddr,
    pub segments: Vec<Segment<'a>>,
}

/// Why a file will not run.
#[derive(Debug)]
pub enum Error {
    /// Not an ELF this crate can read at all.
    Unreadable,
    /// Readable, but not a static RV64 executable: a shared object would need relocating, and
    /// another machine's code would not run.
    NotStaticRiscv64 { kind: u16, machine: u16 },
    /// No program headers, so nothing says what to load.
    NoProgramHeaders,
    /// A program header names bytes outside the file.
    SegmentOutOfFile { offset: u64, size: u64 },
    /// A program header claims more file bytes than it occupies in memory.
    SegmentOverfull { file_size: u64, mem_size: u64 },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable => write!(f, "not a readable ELF"),
            Self::NotStaticRiscv64 { kind, machine } => {
                write!(f, "not a static RV64 executable (e_type {kind}, e_machine {machine})")
            }
            Self::NoProgramHeaders => write!(f, "no program headers"),
            Self::SegmentOutOfFile { offset, size } => {
                write!(f, "a segment names {size:#x} bytes at offset {offset:#x}, past the file")
            }
            Self::SegmentOverfull { file_size, mem_size } => {
                write!(f, "a segment carries {file_size:#x} file bytes in {mem_size:#x} of memory")
            }
        }
    }
}

/// Read `bytes` as an executable, or say why it is not one.
///
/// Every `PT_LOAD` becomes a segment and everything else is ignored: a note, a stack annotation or
/// a section table says nothing about what to put in an address space.
pub fn parse(bytes: &[u8]) -> Result<Image<'_>, Error> {
    let image = ElfBytes::<LittleEndian>::minimal_parse(bytes).map_err(|_| Error::Unreadable)?;

    if image.ehdr.e_type != ET_EXEC || image.ehdr.e_machine != EM_RISCV {
        return Err(Error::NotStaticRiscv64 {
            kind: image.ehdr.e_type,
            machine: image.ehdr.e_machine,
        });
    }

    let headers = image.segments().ok_or(Error::NoProgramHeaders)?;
    let mut segments = Vec::new();
    for header in headers.iter().filter(|header| header.p_type == PT_LOAD) {
        if header.p_filesz > header.p_memsz {
            return Err(Error::SegmentOverfull {
                file_size: header.p_filesz,
                mem_size: header.p_memsz,
            });
        }
        let start = usize::try_from(header.p_offset).map_err(|_| Error::Unreadable)?;
        let len = usize::try_from(header.p_filesz).map_err(|_| Error::Unreadable)?;
        let data = bytes
            .get(start..start + len)
            .ok_or(Error::SegmentOutOfFile { offset: header.p_offset, size: header.p_filesz })?;

        segments.push(Segment {
            vaddr: VirtualAddr::new(header.p_vaddr as usize),
            data,
            bytes: header.p_memsz as usize,
            rights: rights_of(header.p_flags),
        });
    }

    Ok(Image { entry: VirtualAddr::new(image.ehdr.e_entry as usize), segments })
}

/// `p_flags` as page-table rights. The status and `USER` bits belong to whoever maps them.
fn rights_of(flags: u32) -> PteFlags {
    let mut rights = PteFlags::empty();
    if flags & PF_R != 0 {
        rights |= PteFlags::READ;
    }
    if flags & PF_W != 0 {
        rights |= PteFlags::WRITE;
    }
    if flags & PF_X != 0 {
        rights |= PteFlags::EXECUTE;
    }
    rights
}
