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
//! The device list matters in particular: an earlier version of this module mapped
//! "the low gigabyte" as one gigapage, justified by a comment listing the QEMU
//! virt addresses. That was a second, coarser encoding of what the DTB already
//! says, and it mapped 1 GiB `rw-` to cover a few MiB of real registers.
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
//! # Not yet
//!
//! - `GLOBAL` is deliberately not set. It is a TLB optimisation whose correctness
//!   depends on address spaces that do not exist yet.
//! - Device windows are still mapped identity *as well as* into the direct map,
//!   because `console.rs` hands the raw device-tree base to `MmioSerialPort` and
//!   caches the result. That is the next thing to go.
//! - Single-hart only. With SMP, secondary harts must *install* this table rather
//!   than build their own.

use heapless::Vec;
use riscv::register::sstatus;

use paging::sv39::page_size_at;
use paging::utils::{align_down, align_up};
use paging::{LinearOffset, Mapper, PhysicalAddr, PteFlags, Satp, Table, VirtualAddr};

use crate::memory::frame::{self, TableFrames};
use crate::memory::region::{self, Region};
use crate::memory::{direct_map, layout, phys_to_virt, virt_to_phys};

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

/// Bytes mapped by one leaf at the middle level.
const SUPERPAGE: usize = page_size_at(1);

/// Upper bound on the region list: eight kernel regions plus two entries (direct
/// and identity) per device window. Generous, and a compile-time bound beats a
/// heap allocation in the code that builds page tables.
const MAX_REGIONS: usize = 24;

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
fn regions() -> Vec<Region, MAX_REGIONS> {
    let mut regions = Vec::new();
    let mut push = |region: Region| {
        regions.push(region).unwrap_or_else(|_| {
            panic!("kernel layout needs more than {MAX_REGIONS} regions; raise MAX_REGIONS")
        });
    };

    // ---- Device memory, exactly as the device tree describes it ----
    // 4 KiB pages for exactness, not for lack of alignment: a superpage rounds
    // outward, and next to a device register window sits either another device or
    // nothing, neither of which should be mapped by accident. Some windows would
    // in fact fit superpages (QEMU virt's PLIC is 3 aligned MiB), but a few
    // thousand exact leaves cost less than a mapping that reaches past its device.
    for device in crate::device_tree::mmio_regions() {
        let va = phys_to_virt(device.base);
        push(Region {
            name: device.name,
            va,
            pa: device.base,
            len: device.size,
            level: 0,
            flags: READ_WRITE,
        });
        // The same window identity-mapped. Temporary: `console.rs` caches an
        // `MmioSerialPort` built from the raw device-tree base, so dropping this
        // means converting that call site first.
        push(Region {
            name: "  (identity)",
            va: device.base,
            pa: device.base,
            len: device.size,
            level: 0,
            flags: READ_WRITE,
        });
    }

    // ---- The kernel image, one section at a time ----
    push(direct("text", layout::text_start(), layout::text_end(), 0, READ_EXEC));
    push(direct("rodata", layout::rodata_start(), layout::rodata_end(), 0, READ_ONLY));
    push(direct("data", layout::data_start(), layout::data_end(), 0, READ_WRITE));
    push(direct("bss", layout::bss_start(), layout::bss_end(), 0, READ_WRITE));
    push(direct(
        "kernel stack",
        layout::kernel_stack_start(),
        layout::kernel_stack_end(),
        0,
        READ_WRITE,
    ));

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

    regions
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
    // table does, so execution continues across the write. Interrupts are masked
    // so no trap can observe a half-switched translation.
    unsafe {
        let interrupts_were_on = sstatus::read().sie();
        if interrupts_were_on {
            sstatus::clear_sie();
        }
        core::arch::asm!(
            "csrw satp, {satp}",
            "sfence.vma",
            satp = in(reg) satp.bits(),
            options(nostack)
        );
        if interrupts_were_on {
            sstatus::set_sie();
        }
    }

    println!("[memory] kernel page table live (satp {:#x}); boot table retired", satp.bits());
}

/// Require the gaps the layout leaves to really be gaps.
///
/// Only the deliberate one is checked. The linker reserves the page between the
/// kernel stack and the heap explicitly (`_heap_start = _kernel_stack_end + 4096`),
/// so a mapping there would mean the region list had grown over it. The
/// `.rodata`/`.data` alignment slack is also unmapped, but that is incidental
/// geometry and would be a fragile thing to pin.
fn audit_holes(mapper: &KernelMapper<'_>) {
    let guard = layout::kernel_stack_end();
    assert!(
        mapper.entry_of(VirtualAddr::new(guard)).is_none(),
        "the stack/heap guard page at {guard:#x} is mapped; it must stay a hole to catch overruns"
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
