//! The kernel's own page table — the one that replaces the boot table.
//!
//! The architecture boot entry installs a table of 1 GiB `RWX` gigapages.
//! It is exactly enough to get Rust running at high virtual addresses and not one
//! bit more: with a single blanket permission covering all of memory, `.text` is
//! writable and `.rodata` is executable. Paging is on but buying no protection.
//!
//! This module is the *policy* — which regions exist and what rights they get —
//! plus the `satp` switch. The mechanics of installing and auditing a region list
//! live in [`super::region`], which knows nothing about this particular layout.
//!
//! # Nothing here decides a fact it can look up
//!
//! | Fact | Owner |
//! |---|---|
//! | section bounds | the linker, via [`layout`] |
//! | where device memory is | the device tree, via [`crate::device_tree::mmio_regions`] |
//! | which physical memory is ours | [`frame::owned_range`] |
//! | the direct-map base | [`super::direct_map::VA_OFFSET`] |
//!
//! # No identity mapping
//!
//! Unlike the boot table, this one maps nothing at `VA == PA`. Every physical
//! address the kernel dereferences goes through [`super::phys_to_virt`] first —
//! device registers included, which is what the linear direct map bought. The
//! low half of the address space is now entirely unmapped and available to user
//! processes.
//!
//! # One layout, consumed twice
//!
//! [`regions`] computes the list once, and it is used to *install*, then to
//! *audit*, then to *report*. A separate list of expectations would be a second
//! encoding free to drift, and the drift would not surface until something
//! faulted.
//!
//! # Why auditing must precede the switch
//!
//! Installing a table that mis-maps `.text` faults on the instruction *after*
//! `csrw satp`, with the old table already gone: unrecoverable, and nearly
//! undiagnosable. So every page of every region is walked while the boot table is
//! still live, along with the running PC and stack pointer, and the switch happens
//! only if all of it checks out.
//!
//! # One table, adopted rather than rebuilt
//!
//! Secondary harts install *this* table; they do not build their own. Two trees
//! mapping the same kernel would work, but they would double the page-table memory
//! and, worse, drift the moment anything mapped something at run time.
//!
//! They adopt it in the stackless architecture entry because a secondary has to be
//! on this table before it can touch its guarded stack.
//!
//! # Not yet
//!
//! - `GLOBAL` is deliberately not set. It is a TLB optimisation whose correctness
//!   depends on address spaces that do not exist yet.

use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::vec::Vec;

use paging::sv39::{LEVELS, page_size_at};
use paging::{
    LinearOffset, MemoryAddr, Mapper, PhysicalAddr, PteFlags, Satp, Table, VirtualAddr,
};

use crate::memory::frame::{self, TableFrames};
use crate::memory::region::{self, Region};
use crate::memory::stack;
use crate::memory::{SUPERPAGE, direct_map, layout, phys_to_virt, virt_to_phys};

/// This kernel's one mapper flavour: frames from the physical allocator, physical
/// memory reached through the direct map.
type KernelMapper<'a> = Mapper<'a, TableFrames, LinearOffset>;

/// `A` is pre-set on every kernel mapping so the hardware walker never has to
/// write back into a table. `D` is set only where the page is writable, since it
/// means "has been written" and is meaningless on a read-only page.
const READ_ONLY: PteFlags = PteFlags::READ.union(PteFlags::ACCESS);
const READ_EXEC: PteFlags = PteFlags::READ_EXECUTE.union(PteFlags::ACCESS);
const READ_WRITE: PteFlags =
    PteFlags::READ_WRITE.union(PteFlags::ACCESS).union(PteFlags::DIRTY);

// No MAX_REGIONS. The count is four sections, one boot stack, one per secondary hart
// the machine reports, three direct-map pieces and however many MMIO windows the device
// tree describes; two of those are runtime facts, so the bound could not even be
// written. The heap is up by the time this runs, so the list simply grows.

/// The largest page-table level that can tile `[base, base + len)` exactly.
///
/// "Exactly" is the requirement, not "approximately": a superpage rounds outward, so
/// using one that does not divide the window would map whatever sits next to the
/// device. Both the base and the length must be multiples of the page size.
fn largest_level_for(base: PhysicalAddr, len: usize) -> usize {
    (0..LEVELS)
        .rev()
        .find(|&level| {
            let page = page_size_at(level);
            base.is_aligned(page) && len % page == 0
        })
        // Level 0 always fits: every MMIO window is page-aligned and page-sized once
        // `Region::install` has rounded it, and `validate` rejects it otherwise.
        .unwrap_or(0)
}

/// A direct-map region: `VA` and `PA` differ by the fixed offset, so the physical
/// side is *derived* rather than restated and given a chance to disagree.
fn direct(
    name: &'static str,
    start: VirtualAddr,
    end: VirtualAddr,
    level: usize,
    flags: PteFlags,
) -> Region {
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
fn regions() -> Vec<Region> {
    let mut regions = Vec::new();
    let mut push = |region: Region| {
        regions.push(region);
    };

    // ---- Device memory, exactly as the device tree describes it ----
    //
    // Direct map only, no identity alias: everything that touches a device goes through
    // `phys_to_virt`, so a physical address is never dereferenced as one.
    //
    // Every window the tree describes, not just the devices this kernel drives today —
    // that is what makes a new driver work through `phys_to_virt` instead of needing its
    // own base constant.
    for device in crate::device_tree::mmio_regions() {
        // The device tree reports raw integers; this is where they become addresses.
        // Named once and derived from, so the virtual and physical sides cannot be
        // filled in the wrong way round.
        let base = PhysicalAddr::new(device.base);
        push(Region {
            name: "mmio",
            va: phys_to_virt(base),
            pa: base,
            len: device.size,
            // Largest page the window's own geometry permits: QEMU virt's PCI ECAM is
            // 256 MiB, i.e. 128 superpages instead of 65536 pages.
            level: largest_level_for(base, device.size),
            flags: READ_WRITE,
        });
    }

    // ---- The kernel image, one section at a time ----
    push(direct("text", layout::text_start(), layout::text_end(), 0, READ_EXEC));
    push(direct("rodata", layout::rodata_start(), layout::rodata_end(), 0, READ_ONLY));
    push(direct("data", layout::data_start(), layout::data_end(), 0, READ_WRITE));
    push(direct("bss", layout::bss_start(), layout::bss_end(), 0, READ_WRITE));
    // One region per stack. Individually is what leaves the guard pages unmapped: a
    // single region spanning the area would map straight over them.
    //
    // Boot and secondary stacks go through the same loop. A secondary's is not direct
    // mapped — `stack` explains why it is deliberately double mapped — but each stack
    // reports its own `va` and `pa`, so nothing here has to know the difference.
    //
    // Mapped now, before any secondary starts: a starting hart installs this table and
    // *then* sets `sp`.
    for stack in stack::all() {
        push(Region {
            name: stack.name,
            va: stack.bottom(),
            pa: stack.pa(),
            len: stack.len(),
            level: 0,
            flags: READ_WRITE,
        });
    }

    // ---- The direct map, covering exactly what the frame allocator owns ----
    // Asking `frame` rather than re-deriving a RAM extent is the point: the allocator
    // decides which frames it will hand out, and this maps precisely those.
    let (pool_start, pool_end) = frame::owned_range();
    let pool_start_va = phys_to_virt(pool_start);
    let pool_end_va = phys_to_virt(pool_end);
    assert_eq!(
        pool_start_va,
        layout::heap_start(),
        "the frame pool must begin at the top of the kernel image for the two to tile"
    );

    // The image occupies part of one superpage-aligned slot. Everything inside
    // that slot is mapped at 4 KiB so the sections above can carry different
    // rights; everything past it is bulk direct map and gets superpages. The two
    // must not overlap, or the finer mappings would hit `SuperpageInPath`.
    assert!(
        layout::text_start().is_aligned(SUPERPAGE),
        "the kernel image must start superpage-aligned, or its slot would overlap the bulk direct map"
    );
    let slot_end = pool_start_va.align_up(SUPERPAGE);
    // Superpages only reach the last superpage boundary below the end; any
    // remainder is mapped at 4 KiB rather than rounded up past owned memory.
    let bulk_end = pool_end_va.align_down(SUPERPAGE).max(slot_end);

    // Rest of the image's slot: the frame allocator's bitmap and the first of the
    // frames it vends. 4 KiB, because the slot is.
    push(direct("frame pool head", pool_start_va, slot_end, 0, READ_WRITE));
    // Bulk direct map. Never executable — page tables, user pages and DMA buffers
    // live here, and none of it is kernel text.
    push(direct("direct map", slot_end, bulk_end, 1, READ_WRITE));
    // Whatever sub-superpage remainder is left at the top.
    push(direct("direct map tail", bulk_end, pool_end_va, 0, READ_WRITE));

    audit_disjoint(&regions);
    regions
}

/// Require the regions to tile the address space rather than overlap it.
///
/// Two regions covering one page is not a mapping error the hardware can report: the
/// second `install` simply wins, and the loser's rights vanish. So it is checked here,
/// while the list is still just data.
///
/// The specific hazard: the stacks sit *above* the direct map, at
/// [`super::kernel_va_free_start`], and the direct map's own extent is a runtime fact.
/// They cannot collide today, but they are computed in different modules.
///
/// `O(n²)` over ~30 regions, once, at boot.
fn audit_disjoint(regions: &[Region]) {
    for (index, a) in regions.iter().enumerate() {
        if a.is_empty() {
            continue;
        }
        for b in regions[index + 1..].iter().filter(|b| !b.is_empty()) {
            let disjoint = a.va.add(a.len) <= b.va || b.va.add(b.len) <= a.va;
            assert!(
                disjoint,
                "regions '{}' ({:#x}..{:#x}) and '{}' ({:#x}..{:#x}) overlap; one would \
                 silently replace the other's rights",
                a.name,
                a.va,
                a.va.add(a.len),
                b.name,
                b.va,
                b.va.add(b.len)
            );
        }
    }
}

/// Build the kernel's page table, audit it, and switch `satp` to it.
///
/// Call once, on the boot hart, after [`super::init`] — it allocates frames.
pub fn init() {
    let regions = regions();

    // The root table is permanent: `satp` points at it for the kernel's lifetime, so
    // the token is dropped without freeing to pin the frame.
    let root_frame = frame::alloc().expect("no frame for the kernel root page table");
    let root_pa = root_frame.base();
    // SAFETY: a freshly allocated, zeroed, page-aligned frame that this module now
    // owns exclusively and never releases, reachable through the direct map. That
    // makes a unique `'static` borrow of it sound.
    let root: &'static mut Table = unsafe { &mut *phys_to_virt(root_pa).as_mut_ptr::<Table>() };

    let mut mapper = Mapper::new(root, TableFrames, LinearOffset(direct_map::VA_OFFSET));

    for region in &regions {
        region
            .install(&mut mapper)
            .unwrap_or_else(|error| panic!("mapping region '{}' failed: {error}", region.name));
    }
    for region in &regions {
        region.audit(&mapper);
    }

    audit_holes(&mapper);
    audit_live_context(&mapper);

    println!("[memory] kernel page table root at {root_pa:#x}:");
    region::report(&regions);

    let satp = Satp::sv39(root_pa, 0);
    // SAFETY: `root` is a live Sv39 tree, just audited to map every kernel region
    // — the running PC and SP included — to the same physical addresses the boot
    // table does, so execution continues across the write.
    unsafe { switch_to(satp.bits()) };

    // Published last, so a non-zero value means frames, heap, stacks and table are
    // all up. Release, so the tree itself is visible to any hart that reads it —
    // A secondary handoff copies this value and publishes the complete launch state.
    KERNEL_SATP.store(satp.bits(), Ordering::Release);

    println!("[memory] kernel page table live (satp {:#x}); boot table retired", satp.bits());
}

/// The `satp` the boot hart installed. Zero until [`init`] has finished.
///
/// Read through [`satp`] and copied into each secondary handoff.
static KERNEL_SATP: AtomicUsize = AtomicUsize::new(0);

/// The live kernel page table, or `None` before [`init`] has published one.
pub fn satp() -> Option<usize> {
    match KERNEL_SATP.load(Ordering::Acquire) {
        0 => None,
        bits => Some(bits),
    }
}

/// Point `satp` at `bits` and flush the TLB.
///
/// Interrupts are masked across the pair so no trap can observe a half-switched
/// translation. Nothing can fire here today — `sstatus.SIE` is clear — but the mask is
/// the invariant this function needs, not a reaction to a source that happens to exist.
///
/// # Safety
///
/// `bits` must describe a live, correct Sv39 tree that maps the running PC and
/// stack pointer to the same physical addresses the current table does. Otherwise
/// this faults on the very next instruction, with no table left to diagnose it
/// from.
unsafe fn switch_to(bits: usize) {
    crate::arch::riscv64::interrupts::without(|| unsafe {
        core::arch::asm!(
            "csrw satp, {satp}",
            "sfence.vma",
            satp = in(reg) bits,
            options(nostack)
        );
    });
}

/// Require the gaps the layout leaves to really be gaps.
///
/// A guard page only guards if it is genuinely unmapped, and "unmapped" is the one
/// property that is invisible in the region list — it is the *absence* of an entry.
///
/// Only deliberate gaps are checked. The `.rodata`/`.data` alignment slack happens
/// to be unmapped too, but that is incidental geometry and would be a fragile thing
/// to pin.
fn audit_holes(mapper: &KernelMapper<'_>) {
    let unmapped = |va: VirtualAddr| mapper.entry_of(va).is_none();

    // One guard page below every stack. `stack` says which; this only checks them.
    for guard in stack::guards() {
        assert!(
            unmapped(guard),
            "the stack guard page at {guard:#x} is mapped; it must stay a hole or a \
             stack overflow will corrupt its neighbour silently"
        );
    }

    // The linker also reserves a page between the boot stack and the heap
    // (`_heap_start = _boot_stack_end + _page_size`); a mapping there would mean the
    // region list had grown over it.
    let tail = layout::boot_stack_end();
    assert!(
        unmapped(tail),
        "the page at {tail:#x} between the boot stack and the heap is mapped; it must stay a hole"
    );
}

/// Require the addresses the switch itself depends on to survive it, read from the
/// *running* machine rather than assumed.
fn audit_live_context(mapper: &KernelMapper<'_>) {
    let pc: usize;
    let sp: usize;
    // SAFETY: two reads of the current PC and stack pointer into locals.
    unsafe {
        core::arch::asm!("auipc {}, 0", out(reg) pc, options(nomem, nostack));
        core::arch::asm!("mv {}, sp", out(reg) sp, options(nomem, nostack));
    }
    // Both are bare registers coming out of assembly — the one place in this module
    // where an address genuinely enters untyped.
    check_live(mapper, "instruction stream", VirtualAddr::new(pc), PteFlags::EXECUTE);
    check_live(mapper, "stack pointer", VirtualAddr::new(sp), PteFlags::WRITE);
}

/// Require a currently-live address to survive the switch with `needed` rights.
fn check_live(mapper: &KernelMapper<'_>, what: &str, va: VirtualAddr, needed: PteFlags) {
    let (entry, _) = mapper
        .entry_of(va)
        .unwrap_or_else(|| panic!("the running {what} at {va:#x} would be unmapped"));
    assert!(
        entry.flags().contains(needed),
        "the running {what} at {va:#x} would lack {needed:?}"
    );
    assert_eq!(
        mapper.translate(va),
        Some(virt_to_phys(va)),
        "the running {what} at {va:#x} would move to a different frame"
    );
}
