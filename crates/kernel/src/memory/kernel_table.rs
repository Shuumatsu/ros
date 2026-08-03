//! The kernel's own page table — the one that replaces the boot table.
//!
//! `boot.S` installs a table of 1 GiB `RWX` gigapages (see [`super::direct_map`]).
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
//! Every input is owned by whoever actually knows it:
//!
//! | Fact | Owner |
//! |---|---|
//! | section bounds | the linker, via [`layout`] |
//! | where device memory is | the device tree, via [`crate::device_tree::mmio_regions`] |
//! | which physical memory is ours | [`frame::owned_range`] |
//! | the direct-map base | [`super::direct_map::VA_OFFSET`] |
//!
//! The device list has been wrong twice, in opposite directions, so it is worth
//! recording both. First this module mapped "the low gigabyte" as one gigapage,
//! justified by a comment listing the QEMU virt addresses — a coarser second
//! encoding of what the DTB already said. Then `mmio_regions()` replaced it but
//! returned only UART, PLIC and CLINT while *claiming* to be every window, which
//! left a future driver nowhere to look up its own. It is now a real walk of the
//! tree, and this maps all of it.
//!
//! # No identity mapping
//!
//! Unlike `boot.S`'s table, this one maps nothing at `VA == PA`. Every physical
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
//! They adopt it in `boot.S`, reading [`KERNEL_SATP`] directly, because a secondary
//! has to be on this table *before* it can touch its stack — the stack lives above
//! the direct map, in address space no other table describes. There is no handshake:
//! the boot hart cannot start a hart before publishing, since the stack address it
//! passes is only meaningful under the published table.
//!
//! # Not yet
//!
//! - `GLOBAL` is deliberately not set. It is a TLB optimisation whose correctness
//!   depends on address spaces that do not exist yet.

use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::vec::Vec;

use paging::sv39::{LEVELS, page_size_at};
use paging::utils::{align_down, align_up};
use paging::{LinearOffset, Mapper, PhysicalAddr, PteFlags, Satp, Table, VirtualAddr};

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

// No MAX_REGIONS. The count is four sections, one boot stack, one per secondary
// hart the machine reports, three direct-map pieces and however many MMIO windows
// the device tree happens to describe — a bound would be a hand-computed composite
// of all five, in the same class as the `0x110000` that used to sit in kernel.ld,
// and it would silently become too small the moment any of them grew. Two of the
// five are runtime facts now, so it could not even be written. The heap is already
// up by the time this runs (`super::init` adds it before calling here), so the list
// simply grows.

/// The largest page-table level that can tile `[base, base + len)` exactly.
///
/// "Exactly" is the requirement, not "approximately": a superpage rounds outward, so
/// using one that does not divide the window would map whatever sits next to the
/// device. Both the base and the length must be multiples of the page size.
///
/// This is why big apertures are affordable — QEMU virt's PCI ECAM is 256 MiB, which
/// is 128 superpages rather than 65536 pages.
fn largest_level_for(base: usize, len: usize) -> usize {
    (0..LEVELS)
        .rev()
        .find(|&level| {
            let page = page_size_at(level);
            base % page == 0 && len % page == 0
        })
        // Level 0 always fits: every MMIO window is page-aligned and page-sized once
        // `Region::install` has rounded it, and `validate` rejects it otherwise.
        .unwrap_or(0)
}

/// A direct-map region: `VA` and `PA` differ by the fixed offset, so the physical
/// side is *derived* rather than restated and given a chance to disagree.
fn direct(
    name: &'static str,
    start: usize,
    end: usize,
    level: usize,
    flags: PteFlags,
) -> Region {
    Region {
        name,
        va: start,
        pa: virt_to_phys(start),
        len: end.saturating_sub(start),
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
    // Direct map only, no identity alias: everything that touches a device goes
    // through `phys_to_virt` now (`console.rs`, `plic::register`,
    // `device_tree::init`), so a physical address is never dereferenced as one.
    //
    // Every window the tree describes, not a list of the devices this kernel
    // currently drives. Mapping them all is what makes a new driver just work
    // through `phys_to_virt` instead of needing its own base constant, and
    // `largest_level_for` keeps it cheap.
    for device in crate::device_tree::mmio_regions() {
        push(Region {
            name: "mmio",
            va: phys_to_virt(device.base),
            pa: device.base,
            len: device.size,
            // Largest page the window's own geometry permits, so a big aperture does
            // not cost thousands of leaves. QEMU virt's PCI ECAM is 256 MiB: 128
            // superpages instead of 65536 pages.
            level: largest_level_for(device.base, device.size),
            flags: READ_WRITE,
        });
    }

    // ---- The kernel image, one section at a time ----
    push(direct("text", layout::text_start(), layout::text_end(), 0, READ_EXEC));
    push(direct("rodata", layout::rodata_start(), layout::rodata_end(), 0, READ_ONLY));
    push(direct("data", layout::data_start(), layout::data_end(), 0, READ_WRITE));
    push(direct("bss", layout::bss_start(), layout::bss_end(), 0, READ_WRITE));
    // Every kernel stack, one region each. Individually is precisely what leaves the
    // guard pages unmapped: a single region spanning the area would map straight over
    // them, and the overflow protection with them.
    //
    // Boot and secondary stacks go through the same loop and the same `Region`, which
    // is why this asks `stack::all()` rather than assembling the set here. A secondary
    // is not direct mapped — `stack` explains why it is deliberately double mapped —
    // but nothing at this level has to know that: each stack reports its own `va` and
    // `pa` and the difference stops mattering.
    //
    // Mapped now, before any secondary starts, because a starting hart installs this
    // table and *then* sets `sp`. There is no window in which it could fault on a
    // stack that had not been mapped yet.
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
    // Asking `frame` rather than re-deriving a RAM extent is the point: the
    // allocator decides which frames it will hand out, and this maps precisely
    // those, so the two cannot drift into disagreeing about what is reachable.
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
    assert_eq!(
        layout::text_start() % SUPERPAGE,
        0,
        "the kernel image must start superpage-aligned, or its slot would overlap the bulk direct map"
    );
    let slot_end = align_up(pool_start_va, SUPERPAGE);
    // Superpages only reach the last superpage boundary below the end; any
    // remainder is mapped at 4 KiB rather than rounded up past owned memory.
    let bulk_end = align_down(pool_end_va, SUPERPAGE).max(slot_end);

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
/// The specific hazard this exists for: the stacks sit *above* the direct map, at
/// [`super::kernel_va_free_start`], and the direct map's own extent is a runtime fact.
/// They cannot collide today, but they are computed in different modules, and "these
/// two happen not to overlap" is exactly the kind of agreement that holds until
/// someone rounds one of them differently.
///
/// `O(n²)` over ~30 regions, once, at boot.
fn audit_disjoint(regions: &[Region]) {
    for (index, a) in regions.iter().enumerate() {
        if a.is_empty() {
            continue;
        }
        for b in regions[index + 1..].iter().filter(|b| !b.is_empty()) {
            let disjoint = a.va + a.len <= b.va || b.va + b.len <= a.va;
            assert!(
                disjoint,
                "regions '{}' ({:#x}..{:#x}) and '{}' ({:#x}..{:#x}) overlap; one would \
                 silently replace the other's rights",
                a.name,
                a.va,
                a.va + a.len,
                b.name,
                b.va,
                b.va + b.len
            );
        }
    }
}

/// Build the kernel's page table, audit it, and switch `satp` to it.
///
/// Call once, on the boot hart, after [`super::init`] — it allocates frames.
pub fn init() {
    let regions = regions();

    // The root table is permanent: `satp` points at it for the kernel's lifetime.
    // The token is deliberately dropped without freeing, which pins the frame —
    // the same handoff `super::init` makes for the heap.
    let root_frame = frame::alloc().expect("no frame for the kernel root page table");
    let root_pa = root_frame.base();
    // SAFETY: a freshly allocated, zeroed, page-aligned frame that this module now
    // owns exclusively and never releases, reachable through the direct map. That
    // makes a unique `'static` borrow of it sound.
    let root: &'static mut Table =
        unsafe { &mut *(phys_to_virt(root_pa.bits()) as *mut Table) };

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

    println!("[memory] kernel page table root at {:#x}:", root_pa.bits());
    region::report(&regions);

    let satp = Satp::sv39(root_pa, 0);
    // SAFETY: `root` is a live Sv39 tree, just audited to map every kernel region
    // — the running PC and SP included — to the same physical addresses the boot
    // table does, so execution continues across the write.
    unsafe { switch_to(satp.bits()) };

    // Published last, so a non-zero value means frames, heap, stacks and table are
    // all up. Release, so the tree itself is visible to any hart that reads it —
    // `boot.S` pairs this with an acquire fence on the far side.
    KERNEL_SATP.store(satp.bits(), Ordering::Release);

    println!("[memory] kernel page table live (satp {:#x}); boot table retired", satp.bits());
}

/// The `satp` the boot hart installed. Zero until [`init`] has finished.
///
/// `no_mangle` because `boot.S` loads it: a starting hart has to be on this table
/// before it can touch the stack it was given, so the switch happens in assembly,
/// before there is a stack to run Rust on. Reading the same word both places is what
/// keeps assembly and Rust from having separate ideas of which table is live.
///
/// No `#[used]`: [`satp`] reads it from Rust, so it cannot be dropped.
#[unsafe(no_mangle)]
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
/// translation. Nothing can currently fire here — the trap subsystem is parked (see
/// `attic/trap/`) and `sstatus.SIE` is clear — but the mask stays: it is the
/// invariant this function needs, not a reaction to a source that happens to exist,
/// and re-deriving it when interrupts come back is how the window gets reopened.
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
/// So it gets checked directly.
///
/// Only deliberate gaps are checked. The `.rodata`/`.data` alignment slack happens
/// to be unmapped too, but that is incidental geometry and would be a fragile thing
/// to pin.
fn audit_holes(mapper: &KernelMapper<'_>) {
    let unmapped = |va: usize| mapper.entry_of(VirtualAddr::new(va)).is_none();

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
    check_live(mapper, "instruction stream", pc, PteFlags::EXECUTE);
    check_live(mapper, "stack pointer", sp, PteFlags::WRITE);
}

/// Require a currently-live address to survive the switch with `needed` rights.
fn check_live(mapper: &KernelMapper<'_>, what: &str, va: usize, needed: PteFlags) {
    let vaddr = VirtualAddr::new(va);
    let (entry, _) = mapper
        .entry_of(vaddr)
        .unwrap_or_else(|| panic!("the running {what} at {va:#x} would be unmapped"));
    assert!(
        entry.flags().contains(needed),
        "the running {what} at {va:#x} would lack {needed:?}"
    );
    assert_eq!(
        mapper.translate(vaddr),
        Some(PhysicalAddr::new(virt_to_phys(va))),
        "the running {what} at {va:#x} would move to a different frame"
    );
}
