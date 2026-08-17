//! The kernel's own page table, replacing the boot table's blanket `RWX` gigapages with
//! per-section rights, W^X and no identity half.
//!
//! Only the *policy* — which regions exist and what rights they get. Installing and
//! auditing is [`super::region`]'s, the tree itself [`AddressSpace`]'s, and every fact
//! here comes from its owner: [`layout`], the [`MachineMemory`](super::machine::MachineMemory)
//! handed to [`super::init_page_table`], [`frame::owned_range`], [`kernel_va`].
//!
//! [`regions`] is computed once and then installed, audited and reported, so there is no
//! second list to drift. The audit precedes the switch because a mis-mapped `.text` faults
//! on the instruction *after* `csrw satp`, with the old table already gone.
//!
//! `GLOBAL` is unset: a TLB optimisation whose correctness depends on address spaces that
//! do not exist yet.

use alloc::vec::Vec;

use paging::{
    MemoryAddr, PhysicalAddr, PteFlags, SUPERPAGE, Satp, Scheme, VirtualAddr, page_size_at,
};
use spin::Once;

use super::address_space::{AddressSpace, KernelMapper};
use super::direct_map::{phys_to_virt, virt_to_phys};
use super::phys_range::{self, PhysRange};
use super::region::{self, Region};
use super::{KernelScheme, frame, kernel_va, layout, stack};
use crate::arch;
use crate::sync::IrqMutex;

/// One address space, never switched away from, so no id is needed. Named so the zero
/// does not read as a magic argument.
const KERNEL_ASID: usize = 0;

/// `A` is pre-set everywhere so the walker never writes back into a table; `D` only where
/// writable, since it means "has been written".
const READ_ONLY: PteFlags = PteFlags::READ.union(PteFlags::ACCESS);
const READ_EXEC: PteFlags = PteFlags::READ_EXECUTE.union(PteFlags::ACCESS);
const READ_WRITE: PteFlags = PteFlags::READ_WRITE.union(PteFlags::ACCESS).union(PteFlags::DIRTY);

// No MAX_REGIONS: the hart count and the MMIO window count are runtime facts, so the
// bound could not be written. The heap is up by the time this runs.

/// The largest page-table level that tiles `[base, base + len)` without reaching outside
/// its own page.
///
/// A leaf rounds outward, so a level that does not divide the window would map whatever
/// sits next to the device. Level 0 is the floor rather than a further candidate: a
/// sub-page window has no exact tiling at all, and rounding it out to the 4 KiB page it
/// already sits in is the smallest thing a page table can express.
fn largest_level_for(base: PhysicalAddr, len: usize) -> usize {
    (1..KernelScheme::LEVELS)
        .rev()
        .find(|&level| {
            let page = page_size_at(level);
            base.is_aligned(page) && len.is_multiple_of(page)
        })
        .unwrap_or(0)
}

/// A direct-map region: the physical side is *derived* from the virtual one, so the two
/// cannot disagree.
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

/// Compute the kernel's address-space layout.
///
/// `windows` is the coalesced device list rather than the machine's own, so it is passed
/// in: it has to outlive the regions that borrow their names from it.
fn regions<'a>(windows: &'a [PhysRange]) -> Vec<Region<'a>> {
    let mut regions = Vec::new();
    let mut push = |region: Region<'a>| {
        regions.push(region);
    };

    // Every window the machine describes, not just the devices driven today: that is what
    // lets a new driver work through `phys_to_virt` instead of its own base constant.
    // `MachineMemory::check` has already rejected any that the direct map cannot reach or
    // that overlaps RAM. Each keeps its device-tree node name, so the map below says which
    // device a mapping belongs to.
    for device in windows {
        push(Region {
            name: device.name(),
            va: phys_to_virt(device.base),
            pa: device.base,
            len: device.size,
            level: largest_level_for(device.base, device.size),
            flags: READ_WRITE,
        });
    }

    push(direct("text", layout::text_start(), layout::text_end(), 0, READ_EXEC));
    push(direct("rodata", layout::rodata_start(), layout::rodata_end(), 0, READ_ONLY));
    push(direct("data", layout::data_start(), layout::data_end(), 0, READ_WRITE));
    push(direct("bss", layout::bss_start(), layout::bss_end(), 0, READ_WRITE));
    // One region per stack, which is what leaves the guards unmapped — a single region
    // spanning the area would map over them. Each stack reports its own `va` and `pa`,
    // so the secondaries' double mapping needs no special case here.
    for stack in stack::all() {
        push(Region {
            name: stack.name,
            va: stack.bottom(),
            pa: stack.pa(),
            len: stack.bytes(),
            level: 0,
            flags: READ_WRITE,
        });
    }

    // The direct map covers exactly what the allocator owns — asked for, not re-derived —
    // less the kernel image, whose sections above already map that span with rights of
    // their own. The pool is the whole RAM bank, so the image sits inside it rather than
    // above it, and the map is the two spans on either side.
    let pool = frame::owned_range();
    let pool_start_va = phys_to_virt(pool.base);
    let pool_end_va = phys_to_virt(pool.end());
    let image_start = layout::memory_start();
    let image_end = layout::kernel_top();
    assert!(
        pool_start_va <= image_start && image_end <= pool_end_va,
        "the kernel image at {image_start:#x}..{image_end:#x} is not inside the frame pool at \
         {pool_start_va:#x}..{pool_end_va:#x}, so the two tilings below would leave a hole"
    );
    tile_direct_map(&mut push, pool_start_va, image_start);
    tile_direct_map(&mut push, image_end, pool_end_va);

    region::audit_disjoint(&regions);
    audit_kernel_va(&regions);
    regions
}

/// Tile `[start, end)` of the direct map: 4 KiB pages up to the first [`SUPERPAGE`]
/// boundary, superpages through the last one, 4 KiB pages for the remainder. Any of the
/// three may come out empty.
///
/// Three regions rather than one, because a superpage leaf rounds outward: a single
/// superpage region over a span whose ends are not superpage-aligned reaches outside it,
/// onto the kernel image at the wrong rights or onto physical memory that need not exist.
///
/// Never executable: page tables, user pages and DMA buffers live here.
fn tile_direct_map<'a>(push: &mut impl FnMut(Region<'a>), start: VirtualAddr, end: VirtualAddr) {
    if end <= start {
        return;
    }
    let head_end = start.align_up(SUPERPAGE).min(end);
    let bulk_end = end.align_down(SUPERPAGE).max(head_end);
    push(direct("direct map", start, head_end, 0, READ_WRITE));
    push(direct("direct map", head_end, bulk_end, 1, READ_WRITE));
    push(direct("direct map", bulk_end, end, 0, READ_WRITE));
}

/// Require every mapping above the direct map to have come from [`kernel_va`], and
/// everything below to stay below.
///
/// Addresses up there are *chosen*, and the choice has one legitimate source; inventing
/// one is then a boot panic instead of a collision found later by whoever gets mapped
/// over. Both directions, because the boundary is derived in two modules.
fn audit_kernel_va(regions: &[Region<'_>]) {
    let free_start = kernel_va::START;
    for region in regions.iter().filter(|region| !region.is_empty()) {
        // The footprint, not the requested extent: the rounding is what gets mapped, so
        // it is what has to have been reserved.
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

/// The kernel address space, published *after* the switch: a value here means frames,
/// heap, stacks and table are all up, which is what a secondary needs before it adopts the
/// table. The lock holds the kernel's only `&mut` to the root.
static KERNEL: Once<IrqMutex<AddressSpace>> = Once::new();

/// Build the kernel's page table, audit it, and switch `satp` to it.
///
/// Call once, on the boot hart, after [`super::init_allocators`] — it needs frames for the
/// tree and the heap for the region list — and after every stack exists, since a stack
/// allocated later would be memory this table does not map. [`stack::seal`] makes that
/// second condition an error rather than a fault on some hart's first push.
///
/// # Panics
///
/// If a table has already been published. A second call would build and *activate* a second
/// tree while [`satp`] went on reporting the first, which is two answers to which table is
/// live — the one thing this module exists to prevent. `Once` cannot express that on its
/// own: publication has to follow the switch, so the guard is separate from it.
pub fn init(mmio: &[PhysRange]) {
    assert!(
        KERNEL.get().is_none(),
        "kernel_table::init called twice; the live table is already published"
    );
    stack::seal();

    // Merged first: page rounding is this module's, so two windows sharing a page is this
    // module's to absorb rather than `audit_disjoint`'s to reject.
    let windows = phys_range::coalesce(mmio);
    let regions = regions(&windows);
    let mut space = AddressSpace::new(KERNEL_ASID);

    space.edit(|mapper| {
        for region in &regions {
            region
                .install(mapper)
                .unwrap_or_else(|error| panic!("mapping region '{}' failed: {error}", region.name));
        }
    });
    space.walk(|mapper| {
        for region in &regions {
            region.audit(mapper);
        }
        audit_holes(mapper);
        audit_live_context(mapper);
    });

    println!("[memory] kernel page table root at {:#x}:", space.root());
    region::report(&regions);

    let satp = space.satp();
    // SAFETY: a live tree, just audited to map every kernel region — the running
    // PC and SP included — to the same physical addresses the boot table does, so
    // execution continues across the write.
    unsafe { space.activate() };

    KERNEL.call_once(|| IrqMutex::new(space));
    // This hart only. The boot table stays live: every hart started from here on enters
    // through it, because a starting hart has no translation of its own to arrive with.
    println!("[memory] kernel page table live on this hart (satp {:#x})", satp.bits());
}

/// Edit or walk the kernel address space; the way in for anything that maps after boot.
///
/// `None` until [`init`] has finished — during it the space is still a local, so a
/// half-built table is unreachable.
pub fn with<R>(f: impl FnOnce(&mut AddressSpace) -> R) -> Option<R> {
    KERNEL.get().map(|space| space.with(f))
}

/// The live kernel page table's `satp`, copied into each secondary handoff.
///
/// Read out of the address space rather than mirrored in a static: two copies could
/// disagree about which table is live. Typed, because the register value that decides
/// whether a hart boots is the last thing that should cross a subsystem boundary as a
/// bare integer.
pub fn satp() -> Option<Satp> { with(|space| space.satp()) }

/// Require the deliberate gaps to really be gaps.
///
/// A guard only guards if it is unmapped, which is the one property a region list cannot
/// express. Only the deliberate ones: section alignment slack is unmapped too, but
/// incidentally.
fn audit_holes(mapper: &KernelMapper<'_>) {
    let unmapped = |va: VirtualAddr| mapper.entry_of(va).is_none();

    for guard in stack::guards() {
        assert!(
            unmapped(guard),
            "the stack guard page at {guard:#x} is mapped; it must stay a hole or a \
             stack overflow will corrupt its neighbour silently"
        );
    }

    // The linker's page between the boot stack and free RAM. A mapping there means the
    // region list has grown over it.
    let tail = layout::boot_stack_end();
    assert!(
        unmapped(tail),
        "the page at {tail:#x} between the boot stack and the heap is mapped; it must stay a hole"
    );
}

/// Require the addresses the switch depends on to survive it, read from the *running*
/// machine rather than assumed.
fn audit_live_context(mapper: &KernelMapper<'_>) {
    check_live(mapper, "instruction stream", arch::pc(), PteFlags::EXECUTE);
    check_live(mapper, "stack pointer", arch::sp(), PteFlags::WRITE);
}

/// Require a currently-live address to survive the switch with `needed` rights.
fn check_live(mapper: &KernelMapper<'_>, what: &str, va: VirtualAddr, needed: PteFlags) {
    let (entry, _) = mapper
        .entry_of(va)
        .unwrap_or_else(|| panic!("the running {what} at {va:#x} would be unmapped"));
    assert!(entry.flags().contains(needed), "the running {what} at {va:#x} would lack {needed:?}");
    assert_eq!(
        mapper.translate(va),
        Some(virt_to_phys(va)),
        "the running {what} at {va:#x} would move to a different frame"
    );
}
