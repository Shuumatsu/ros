//! User image page-table policy.
//!
//! User spaces share the kernel table's upper half, allowing traps without a trampoline page.

use alloc::vec::Vec;

use mmu::{MemoryAddr, PAGE_SIZE, PteFlags, VirtualAddr};

use super::address_space::AddressSpace;
use super::direct_map::phys_to_virt;
use super::region::{self, Region};
use super::stack::Stack;
use super::{frame, kernel_table, user};
use crate::arch;

const USER_ASID: usize = 1;

/// Initial user `sp` and exclusive stack top.
pub const STACK_TOP: VirtualAddr = user::END;

const STACK_SIZE: usize = 4 * PAGE_SIZE;

/// One loadable executable segment.
pub struct Segment<'a> {
    pub vaddr: VirtualAddr,
    pub data: &'a [u8],
    /// In-memory size, which must be at least [`data`](Self::data). Uncopied bytes remain zero.
    pub bytes: usize,
    /// Requested R/W/X rights; `USER` and status bits are added internally.
    pub rights: PteFlags,
}

/// Build a user address space containing `segments`, a user stack, and the shared kernel half.
///
/// `kernel_stack` is the trap stack and must be visible through the shared half.
///
/// # Panics
///
/// Panics if the kernel table or frames are unavailable, page-rounded segments overlap, or an
/// audit fails.
pub fn build(segments: &[Segment<'_>], kernel_stack: &Stack) -> AddressSpace {
    let mut space = AddressSpace::new(USER_ASID);
    kernel_table::with(|kernel| space.share_upper_half_from(kernel))
        .expect("user_table::build before the kernel page table was published");

    let mut regions: Vec<Region<'static>> = Vec::with_capacity(segments.len() + 1);
    regions.extend(segments.iter().map(load));
    regions.push(stack_region());
    region::audit_disjoint(&regions);

    space.edit(|mapper| region::install_all(mapper, &regions));
    space.walk(|mapper| {
        region::audit_all(mapper, &regions);

        // Trap entry and return state must survive the shared-half switch.
        for (what, va) in [
            ("trap vector", arch::trap::vector()),
            ("process kernel stack", kernel_stack.top().sub(1)),
            ("stack pointer", arch::sp()),
        ] {
            assert!(
                mapper.translate(va).is_some(),
                "the user table does not map the {what} at {va:#x}; the kernel half did not come \
                 across"
            );
        }
    });

    println!("[memory] user page table root at {:#x}:", space.root());
    region::report(&regions);
    space
}

/// Allocate and initialize one segment; bytes after `data` remain zero.
fn load(segment: &Segment<'_>) -> Region<'static> {
    let (start, end) = segment.vaddr.footprint(segment.vaddr.add(segment.bytes), PAGE_SIZE);
    let offset = segment.vaddr.sub_addr(start);
    let span = end.sub_addr(start);

    let frames = frame::alloc_contiguous(span / PAGE_SIZE)
        .unwrap_or_else(|| panic!("no contiguous RAM for a {span:#x}-byte user segment"));
    let base = frames.base();

    // SAFETY: the segment-size contract makes the exclusive writable frame span contain `data`.
    unsafe {
        core::ptr::copy_nonoverlapping(
            segment.data.as_ptr(),
            phys_to_virt(base).add(offset).as_mut_ptr::<u8>(),
            segment.data.len(),
        );
    }

    Region {
        name: name_for(segment.rights),
        va: start,
        pa: frames.leak(),
        len: span,
        level: 0,
        flags: region::leaf_flags(segment.rights | PteFlags::USER),
    }
}

fn stack_region() -> Region<'static> {
    let frames = frame::alloc_contiguous(STACK_SIZE / PAGE_SIZE)
        .unwrap_or_else(|| panic!("no contiguous RAM for a {STACK_SIZE:#x}-byte user stack"));

    Region {
        name: "user stack",
        va: STACK_TOP.sub(STACK_SIZE),
        pa: frames.leak(),
        len: STACK_SIZE,
        level: 0,
        flags: region::leaf_flags(PteFlags::USER_READ_WRITE),
    }
}

fn name_for(rights: PteFlags) -> &'static str {
    match (
        rights.contains(PteFlags::WRITE),
        rights.contains(PteFlags::EXECUTE),
        rights.contains(PteFlags::READ),
    ) {
        (false, true, _) => "user text",
        (true, false, _) => "user data",
        (false, false, true) => "user rodata",
        _ => "user segment",
    }
}
