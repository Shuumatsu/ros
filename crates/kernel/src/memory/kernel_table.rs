//! Final kernel page-table policy.
//!
//! The table removes the identity map, enforces W^X, and is audited before activation
//! because an invalid live mapping faults immediately after the `satp` write.

use alloc::vec::Vec;
use core::slice;

use mmu::{MemoryAddr, PhysicalAddr, PteFlags, SUPERPAGE, Satp, Scheme, VirtualAddr, page_size_at};
use spin::Once;

use super::address_space::{AddressSpace, KernelMapper};
use super::direct_map::{phys_to_virt, virt_to_phys};
use super::phys_range::{self, PhysRange};
use super::region::{self, Region};
use super::stack::Stack;
use super::{KernelScheme, frame, kernel_va, layout, stack};
use crate::arch;
use crate::sync::IrqMutex;

const KERNEL_ASID: usize = 0;

const READ_ONLY: PteFlags = region::leaf_flags(PteFlags::READ);
const READ_EXEC: PteFlags = region::leaf_flags(PteFlags::READ_EXECUTE);
const READ_WRITE: PteFlags = region::leaf_flags(PteFlags::READ_WRITE);

/// The largest level that tiles the range exactly; larger leaves could map adjacent memory.
fn largest_level_for(base: PhysicalAddr, len: usize) -> usize {
    (1..KernelScheme::LEVELS)
        .rev()
        .find(|&level| {
            let page = page_size_at(level);
            base.is_aligned(page) && len.is_multiple_of(page)
        })
        .unwrap_or(0)
}

fn direct<'a>(
    name: &'a str,
    start: VirtualAddr,
    end: VirtualAddr,
    level: usize,
    flags: PteFlags,
) -> Region<'a> {
    Region {
        name,
        va: start,
        pa: virt_to_phys(start),
        len: end.checked_sub_addr(start).unwrap_or(0),
        level,
        flags,
    }
}

fn regions<'a>(windows: &'a [PhysRange], stacks: &[Stack]) -> Vec<Region<'a>> {
    let mut regions = Vec::new();

    for device in windows {
        regions.push(direct(
            device.name(),
            phys_to_virt(device.base),
            phys_to_virt(device.end()),
            largest_level_for(device.base, device.size),
            READ_WRITE,
        ));
    }

    regions.push(direct("text", layout::text_start(), layout::text_end(), 0, READ_EXEC));
    regions.push(direct("rodata", layout::rodata_start(), layout::rodata_end(), 0, READ_ONLY));
    regions.push(direct("data", layout::data_start(), layout::data_end(), 0, READ_WRITE));
    regions.push(direct("bss", layout::bss_start(), layout::bss_end(), 0, READ_WRITE));
    // Separate regions leave each guard page unmapped.
    for stack in stacks {
        regions.push(stack_region(stack));
    }

    // Exclude the image because its sections require stricter permissions.
    let pool = frame::owned_range();
    let pool_start_va = phys_to_virt(pool.base);
    let pool_end_va = phys_to_virt(pool.end());
    let (image_start, image_end) = layout::image();
    assert!(
        pool_start_va <= image_start && image_end <= pool_end_va,
        "the kernel image at {image_start:#x}..{image_end:#x} is not inside the frame pool at \
         {pool_start_va:#x}..{pool_end_va:#x}, so the two tilings below would leave a hole"
    );
    tile_direct_map(&mut regions, pool_start_va, image_start);
    tile_direct_map(&mut regions, image_end, pool_end_va);

    region::audit_disjoint(&regions);
    audit_kernel_va(&regions);
    regions
}

/// Tile the direct map without letting superpage rounding cross either endpoint.
fn tile_direct_map<'a>(out: &mut Vec<Region<'a>>, start: VirtualAddr, end: VirtualAddr) {
    if end <= start {
        return;
    }
    let head_end = start.align_up(SUPERPAGE).min(end);
    let bulk_end = end.align_down(SUPERPAGE).max(head_end);
    out.push(direct("direct map", start, head_end, 0, READ_WRITE));
    out.push(direct("direct map", head_end, bulk_end, 1, READ_WRITE));
    out.push(direct("direct map", bulk_end, end, 0, READ_WRITE));
}

/// Require mappings above the direct map to be reserved through [`kernel_va`].
fn audit_kernel_va(regions: &[Region<'_>]) {
    let free_start = kernel_va::START;
    for region in regions.iter().filter(|region| !region.is_empty()) {
        // Page rounding is part of the reservation invariant.
        let (start, end) = region.footprint();
        if start < free_start {
            assert!(
                end <= free_start,
                "region '{}' ({start:#x}..{end:#x}) runs past {free_start:#x}, where the kernel \
                 VA allocator starts handing addresses out",
                region.name
            );
            continue;
        }
        assert!(
            kernel_va::is_reserved(start, end.sub_addr(start)),
            "region '{}' ({start:#x}..{end:#x}) is above the direct map but was never reserved \
             from kernel_va, whose watermark is {:#x}",
            region.name,
            kernel_va::watermark()
        );
    }
}

/// Published after activation; the lock owns the only mutable root reference.
static KERNEL: Once<IrqMutex<AddressSpace>> = Once::new();

/// Build the kernel's page table, audit it, and switch `satp` to it.
///
/// Call once on the boot hart after allocators and all boot-time stacks are ready.
///
/// # Panics
///
/// Panics on repeated initialization or any invalid mapping.
pub fn init(mmio: &[PhysRange]) {
    assert!(
        KERNEL.get().is_none(),
        "kernel_table::init called twice; the live table is already published"
    );

    // One snapshot drives the mappings and the guard-hole audit alike.
    let stacks: Vec<Stack> = stack::all().collect();
    let windows = phys_range::coalesce(mmio);
    let regions = regions(&windows, &stacks);
    let mut space = AddressSpace::new(KERNEL_ASID);

    space.edit(|mapper| region::install_all(mapper, &regions));
    space.walk(|mapper| {
        region::audit_all(mapper, &regions);
        audit_holes(mapper, &stacks);
        audit_live_context(mapper);
    });

    println!("[memory] kernel page table root at {:#x}:", space.root());
    region::report(&regions);

    let satp = space.satp();
    // SAFETY: the audit verified the live PC and SP retain their physical mappings.
    unsafe { space.activate() };

    KERNEL.call_once(|| IrqMutex::new(space));
    // The boot table remains the entry table for harts that have not started.
    println!("[memory] kernel page table live on this hart (satp {:#x})", satp.bits());
}

/// Activate the kernel table on the calling hart.
///
/// # Panics
///
/// Panics before [`init`].
pub fn activate() {
    with(|space| {
        // SAFETY: user tables share the kernel half containing the live PC and SP.
        unsafe { space.activate() };
    })
    .expect("kernel_table::activate before the kernel page table was published");
}

/// Access the published address space, or return `None` before [`init`] completes.
pub fn with<R>(f: impl FnOnce(&mut AddressSpace) -> R) -> Option<R> {
    KERNEL.get().map(|space| space.with(f))
}

fn stack_region(stack: &Stack) -> Region<'static> {
    Region {
        name: stack.name,
        va: stack.bottom(),
        pa: stack.pa(),
        len: stack::SIZE,
        level: 0,
        flags: READ_WRITE,
    }
}

/// Map and audit a runtime kernel stack.
///
/// The mapping is immediately usable only on the calling hart; other harts may have cached its
/// absence and are not fenced by [`AddressSpace::edit`].
///
/// # Panics
///
/// Panics before initialization, on mapping failure, or if the guard page is mapped.
pub(in crate::memory) fn map_stack(stack: &Stack) {
    let region = stack_region(stack);
    with(|space| {
        // User tables copy root slots, so runtime stacks cannot introduce a new upper-half slot.
        assert!(
            space.root_slot(stack.bottom()).is_valid(),
            "stack '{}' at {:#x} falls in a root slot the kernel table has not opened; the address \
             spaces sharing this table's upper half would not see it",
            stack.name,
            stack.bottom()
        );

        let regions = slice::from_ref(&region);
        space.edit(|mapper| region::install_all(mapper, regions));
        space.walk(|mapper| {
            region::audit_all(mapper, regions);
            assert!(
                mapper.translate(stack.guard()).is_none(),
                "the guard page at {:#x} below stack '{}' came out mapped; it must stay a hole \
                 or an overflow corrupts its neighbour silently",
                stack.guard(),
                stack.name
            );
        });
    })
    .expect("kernel_table::map_stack before the kernel table was published");
}

/// Return the live kernel table's `satp`.
pub fn satp() -> Option<Satp> { with(|space| space.satp()) }

/// Verify all deliberate guard gaps remain unmapped.
fn audit_holes(mapper: &KernelMapper<'_>, stacks: &[Stack]) {
    let unmapped = |va: VirtualAddr| mapper.entry_of(va).is_none();

    for guard in stacks.iter().map(Stack::guard) {
        assert!(
            unmapped(guard),
            "the stack guard page at {guard:#x} is mapped; it must stay a hole or a \
             stack overflow will corrupt its neighbour silently"
        );
    }

    let tail = layout::boot_stack_end();
    assert!(
        unmapped(tail),
        "the page at {tail:#x} between the boot stack and the heap is mapped; it must stay a hole"
    );
}

/// Verify the running PC and SP survive the table switch.
fn audit_live_context(mapper: &KernelMapper<'_>) {
    check_live(mapper, "instruction stream", arch::pc(), PteFlags::EXECUTE);
    check_live(mapper, "stack pointer", arch::sp(), PteFlags::WRITE);
}

fn check_live(mapper: &KernelMapper<'_>, what: &str, va: VirtualAddr, needed: PteFlags) {
    let (entry, level) = mapper
        .entry_of(va)
        .unwrap_or_else(|| panic!("the running {what} at {va:#x} would be unmapped"));
    assert!(entry.flags().contains(needed), "the running {what} at {va:#x} would lack {needed:?}");
    assert_eq!(
        entry.phys_at(va, level),
        virt_to_phys(va),
        "the running {what} at {va:#x} would move to a different frame"
    );
}
