//! Which mappings a user image gets, and the address space it gets them in.
//!
//! [`super::kernel_table`]'s counterpart on the other side of the privilege boundary: the same
//! [`Region`] machinery, the same audit, a different policy. Every leaf carries `USER`, and the
//! kernel's half is *shared in* rather than built — so a trap taken in user mode lands on a vector
//! and a kernel stack the running table already maps, and this kernel needs no trampoline page.
//!
//! What a segment is comes from whoever read the executable. Nothing here parses anything. What a
//! process's *stack* is has no image to come from, so it is this module's: the image says where its
//! own pieces go, and everything else in the low half is the kernel's to place.

use alloc::vec::Vec;

use mmu::{MemoryAddr, PAGE_SIZE, PteFlags, VirtualAddr};

use super::address_space::AddressSpace;
use super::direct_map::phys_to_virt;
use super::region::{self, Region};
use super::stack::Stack;
use super::{frame, kernel_table, user};
use crate::arch;

/// The address space id a user image gets. One process at a time, so the id is a constant rather
/// than something allocated.
const USER_ASID: usize = 1;

/// A process's initial `sp`, and one past the top of its stack.
///
/// The top of the user half, so the stack grows down from as far as it can get from the image at
/// the bottom: everything a process may come to want in between — a heap, a mapping of its own —
/// has the whole span to grow into. Nothing is mapped below the lowest stack page, so an overflow
/// faults rather than reaching whatever was placed next.
pub const STACK_TOP: VirtualAddr = user::END;

/// Stack bytes a process gets, which is what a program with no recursion and no buffers of its own
/// needs.
const STACK_SIZE: usize = 4 * PAGE_SIZE;

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

/// Build an address space for `segments`: the kernel's half, one region per segment, and a stack.
///
/// `kernel_stack` is the stack a trap from this process will land on. It belongs to the half that
/// is shared in rather than built, so the audit below is what proves it came across — a stack the
/// user table did not map is a first trap with nowhere to push and nothing left to report from.
///
/// # Panics
///
/// If the kernel table is not live yet, if there are no frames for a segment or the stack, if two
/// segments share a page, or if a mapping does not audit.
pub fn build(segments: &[Segment<'_>], kernel_stack: &Stack) -> AddressSpace {
    let mut space = AddressSpace::new(USER_ASID);
    kernel_table::with(|kernel| space.share_upper_half_from(kernel))
        .expect("user_table::build before the kernel page table was published");

    let mut regions: Vec<Region<'static>> = segments.iter().map(load).collect();
    regions.push(stack_region());
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
        // vector and pushes onto the process's kernel stack, and returns from the stack it was
        // taken on — all three live in the half that was shared rather than built, so these are
        // the checks that say the sharing worked. The stack top is one past the last mapped byte,
        // which is why the address asked about is the byte below it.
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

    Region {
        name: name_for(segment.rights),
        va: start,
        pa: frames.leak(),
        len: span,
        level: 0,
        flags: leaf_flags(segment.rights),
    }
}

/// The mapping a process's stack gets: [`STACK_SIZE`] bytes below [`STACK_TOP`], writable and
/// never executable.
///
/// The frames arrive zeroed from [`frame::alloc_contiguous`], which is all the initialization a
/// stack needs.
fn stack_region() -> Region<'static> {
    let frames = frame::alloc_contiguous(STACK_SIZE / PAGE_SIZE)
        .unwrap_or_else(|| panic!("no contiguous RAM for a {STACK_SIZE:#x}-byte user stack"));

    Region {
        name: "user stack",
        va: STACK_TOP.sub(STACK_SIZE),
        pa: frames.leak(),
        len: STACK_SIZE,
        level: 0,
        flags: leaf_flags(PteFlags::READ_WRITE),
    }
}

/// `rights` as a leaf in a user address space: `USER`, plus the status bits every one of them
/// carries.
///
/// `A` is pre-set everywhere so the hardware never writes back into the table, and `D` wherever a
/// write is allowed — it means "has been written", and the first write will not be observed.
fn leaf_flags(rights: PteFlags) -> PteFlags {
    let status = if rights.contains(PteFlags::WRITE) {
        PteFlags::ACCESS.union(PteFlags::DIRTY)
    } else {
        PteFlags::ACCESS
    };
    rights | PteFlags::USER | status
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
