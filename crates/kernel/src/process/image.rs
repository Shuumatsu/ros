//! Static RV64 ELF loading.

use alloc::vec::Vec;
use core::fmt;

use elf::ElfBytes;
use elf::abi::{EM_RISCV, ET_EXEC, PF_R, PF_W, PF_X, PT_LOAD};
use elf::endian::LittleEndian;
use mmu::{PteFlags, VirtualAddr};

use crate::memory::user_table::Segment;

pub struct Image<'a> {
    pub entry: VirtualAddr,
    pub segments: Vec<Segment<'a>>,
}

#[derive(Debug)]
pub enum Error {
    Unreadable,
    NotStaticRiscv64 { kind: u16, machine: u16 },
    NoProgramHeaders,
    SegmentOutOfFile { offset: u64, size: u64 },
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

/// Parses each `PT_LOAD` segment and ignores other program headers.
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
