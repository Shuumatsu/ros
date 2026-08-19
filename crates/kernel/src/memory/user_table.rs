//! Which mappings a user image gets, and the address space it gets them in.
//!
//! [`super::kernel_table`]'s counterpart on the other side of the privilege boundary: the same
//! [`Region`] machinery, the same audit, a different policy. Every leaf carries `USER`, and the
//! kernel's half is *shared in* rather than built — so a trap taken in user mode lands on a vector
//! the running table already maps, and this kernel needs no trampoline page.
//!
//! What a segment is comes from whoever read the executable. Nothing here parses anything.

use alloc::vec::Vec;

use mmu::{MemoryAddr, PAGE_SIZE, PteFlags, VirtualAddr};

use super::address_space::AddressSpace;
use super::direct_map::phys_to_virt;
use super::region::{self, Region};
use super::{frame, kernel_table};
use crate::arch;

/// The address space id a user image gets. One process at a time, so the id is a constant rather
/// than something allocated.
const USER_ASID: usize = 1;

/// One loadable piece of an executable, as this kernel needs it.
pub struct Segment<'a> {
    /// Where the image asks to be placed. Page alignment is not required.
    pub vaddr: VirtualAddr,
    /// The bytes to copy in.
    pub data: &'a [u8],
    /// Bytes the segment occupies once loaded. Everything past [`data`](Self::data) is zero.
    pub bytes: usize,
    /// Read, write and execute as the image asks for them. `USER` and the status bits are this
    /// module's to add, so no image can ask for a page the kernel would keep to itself.
    pub rights: PteFlags,
}

/// Build an address space for `segments`: the kernel's half, plus one region per segment.
///
/// # Panics
///
/// If the kernel table is not live yet, if there are no frames for a segment, if two segments
/// share a page, or if a mapping does not audit.
pub fn build(segments: &[Segment<'_>]) -> AddressSpace {
    let mut space = AddressSpace::new(USER_ASID);
    kernel_table::with(|kernel| space.share_upper_half_from(kernel))
        .expect("user_table::build before the kernel page table was published");

    let regions: Vec<Region<'static>> = segments.iter().map(load).collect();
    region::audit_disjoint(&regions);

    space.edit(|mapper| {
        for region in &regions {
            region
                .install(mapper)
                .unwrap_or_else(|error| panic!("mapping '{}' failed: {error}", region.name));
        }
    });
    space.walk(|mapper| {
        for region in &regions {
            region.audit(mapper);
        }

        // The kernel's half really came across. A trap taken while this table is live lands on the
        // vector and pushes onto a kernel stack, both of which live in the half that was shared
        // rather than built — so this is the check that says the sharing worked.
        for (what, va) in
            [("trap vector", arch::trap::vector()), ("stack pointer", arch::sp())]
        {
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

/// Give one segment frames, fill them, and describe the mapping it needs.
///
/// The frames arrive zeroed from [`frame::alloc_contiguous`], which is the whole of the segment's
/// zero tail: copying `data` and stopping leaves the rest as the image asks for it.
///
/// Contiguous per segment, so one region describes it. The allocator is buddy-backed, so a segment
/// whose page count is not a power of two strands the difference inside the run it leaks.
fn load(segment: &Segment<'_>) -> Region<'static> {
    let start = segment.vaddr.align_down(PAGE_SIZE);
    let offset = segment.vaddr.sub_addr(start);
    let span = (offset + segment.bytes).next_multiple_of(PAGE_SIZE);

    let frames = frame::alloc_contiguous(span / PAGE_SIZE)
        .unwrap_or_else(|| panic!("no contiguous RAM for a {span:#x}-byte user segment"));
    let base = frames.base();

    // SAFETY: frames this call owns exclusively, reachable and writable through the direct map,
    // and `span` covers `offset + segment.bytes`, so `data` fits inside them.
    unsafe {
        core::ptr::copy_nonoverlapping(
            segment.data.as_ptr(),
            phys_to_virt(base).add(offset).as_mut_ptr::<u8>(),
            segment.data.len(),
        );
    }

    let writable = segment.rights.contains(PteFlags::WRITE);
    let mut flags = segment.rights | PteFlags::USER | PteFlags::ACCESS;
    if writable {
        // `D` means "has been written", and the loader just did.
        flags |= PteFlags::DIRTY;
    }

    Region { name: name_for(segment.rights), va: start, pa: frames.leak(), len: span, level: 0, flags }
}

/// What to call a segment in the boot log, from the only thing an ELF says about it.
///
/// Program headers carry rights, not names — the names are a section-table property a loader has
/// no business reading — so the rights are what there is to report.
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
