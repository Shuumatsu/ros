//! The kernel's own page table — the one that replaces the boot table.
//!
//! `boot.S` installs a table of 1 GiB `RWX` gigapages (see [`super::direct_map`]).
//! It is exactly enough to get Rust running at high virtual addresses and not one
//! bit more: with a single blanket permission covering all of memory, `.text` is
//! writable and `.rodata` is executable. Paging is on but buying no protection.
//!
//! This module builds the real thing — per-section rights, W^X, superpages where
//! they fit, and guard pages where a gap is useful — then switches `satp` to it.
//!
//! # One layout, used twice
//!
//! [`regions`] computes the layout once, and it is consumed **twice**: to install
//! the mappings, and to verify them. That is deliberate. A separate list of
//! expectations would be a second encoding of the same layout, free to drift from
//! the first, and the drift would not surface until something faulted. Checking
//! against the same data instead proves the thing that can actually go wrong —
//! that the mapper did what the layout says.
//!
//! # Why verification must precede the switch
//!
//! Installing a table that mis-maps `.text` faults on the instruction *after*
//! `csrw satp`, with the old table already gone: unrecoverable, and nearly
//! undiagnosable. So every page of every region is walked with
//! [`Mapper::entry_of`] while the boot table is still live, along with the running
//! PC and stack pointer, and the switch happens only if all of it checks out.
//!
//! # Not yet
//!
//! - `GLOBAL` is deliberately not set. It is a TLB optimisation whose correctness
//!   depends on address spaces that do not exist yet; it belongs with user paging.
//! - One identity mapping survives, for MMIO only, because `console.rs` still
//!   treats the device-tree UART base as a pointer. That is the next thing to go.
//! - Single-hart only. With SMP, secondary harts must *install* this table rather
//!   than build their own; the root would need publishing for that.

use riscv::register::sstatus;

use paging::sv39::{ROOT_LEVEL, page_size_at};
use paging::utils::{GIGABYTE, KILOBYTE, MEGABYTE, align_down, align_up};
use paging::{
    LinearOffset, MapError, Mapper, PhysicalAddr, PteFlags, Satp, Table, VirtualAddr,
};

use crate::memory::frame::{self, TableFrames};
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
/// Bytes mapped by one leaf at the root level.
const GIGAPAGE: usize = page_size_at(ROOT_LEVEL);

/// One contiguous mapping: a virtual range, the physical range behind it, the
/// page size to build it from, and the rights it carries.
#[derive(Clone, Copy)]
struct Region {
    name: &'static str,
    va: usize,
    pa: usize,
    len: usize,
    level: usize,
    flags: PteFlags,
}

impl Region {
    fn page_size(&self) -> usize {
        page_size_at(self.level)
    }

    /// Pages installed, counting a partial final page as a whole one.
    fn pages(&self) -> usize {
        self.len.div_ceil(self.page_size())
    }
}

/// A direct-map region: `VA` and `PA` differ by the fixed offset, so the physical
/// side is *derived* here rather than restated and given a chance to disagree.
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

/// Number of entries [`regions`] returns. Some may be empty (`len == 0`) on a
/// given platform; empty regions are skipped rather than special-cased away.
const REGION_COUNT: usize = 10;

/// Compute the kernel's address-space layout.
///
/// Every fact comes from somewhere authoritative: section bounds from the linker
/// through [`layout`], the RAM top from the device tree, the direct-map base and
/// window from [`direct_map`]. No address is written down here.
fn regions() -> [Region; REGION_COUNT] {
    let ram_top = phys_to_virt(
        crate::device_tree::ram_end().expect("device tree RAM top not discovered"),
    );

    // The kernel image occupies part of one superpage-aligned slot. Everything
    // inside that slot is mapped at 4 KiB so the sections can carry different
    // rights; everything above it is bulk direct map and gets superpages. The two
    // passes must not overlap, or the finer one would hit `SuperpageInPath`.
    let slot_end = align_up(layout::heap_start(), SUPERPAGE);
    assert_eq!(
        layout::text_start() % SUPERPAGE,
        0,
        "the kernel image must start superpage-aligned, or its slot would overlap the bulk direct map"
    );

    // The bulk pass can only use superpages up to the last superpage boundary
    // below the RAM top; any remainder is mapped at 4 KiB rather than rounded up
    // past the end of real memory.
    let bulk_end = align_down(ram_top, SUPERPAGE).max(slot_end);

    [
        // Every device we know of lives in the low gigabyte (QEMU virt: UART
        // 0x1000_0000, PLIC 0xc00_0000, CLINT 0x200_0000), so one root leaf covers
        // the lot — R+W and, unlike the boot table, explicitly not executable.
        Region {
            name: "mmio",
            va: phys_to_virt(0),
            pa: 0,
            len: GIGAPAGE,
            level: ROOT_LEVEL,
            flags: READ_WRITE,
        },
        // The same gigabyte identity-mapped. Temporary: `console.rs` hands the raw
        // device-tree UART base to `MmioSerialPort` and caches the result, so
        // dropping this mapping means converting that call site first.
        Region {
            name: "mmio (identity, temporary)",
            va: 0,
            pa: 0,
            len: GIGAPAGE,
            level: ROOT_LEVEL,
            flags: READ_WRITE,
        },
        direct("text", layout::text_start(), layout::text_end(), 0, READ_EXEC),
        direct("rodata", layout::rodata_start(), layout::rodata_end(), 0, READ_ONLY),
        direct("data", layout::data_start(), layout::data_end(), 0, READ_WRITE),
        direct("bss", layout::bss_start(), layout::bss_end(), 0, READ_WRITE),
        direct(
            "kernel stack",
            layout::kernel_stack_start(),
            layout::kernel_stack_end(),
            0,
            READ_WRITE,
        ),
        // The rest of the image's slot: the frame allocator's bitmap and the first
        // of the frames it vends. 4 KiB, because the slot is.
        direct("frame pool head", layout::heap_start(), slot_end, 0, READ_WRITE),
        // Bulk direct map. Never executable — this is where user pages, page
        // tables and DMA buffers live, and none of it is kernel text.
        direct("direct map", slot_end, bulk_end, 1, READ_WRITE),
        // Superpage-sized remainder at the RAM top, if the RAM top is not aligned.
        direct("direct map tail", bulk_end, ram_top, 0, READ_WRITE),
    ]
}

/// Install every page of `region`.
fn install(mapper: &mut KernelMapper<'_>, region: &Region) -> Result<(), MapError> {
    let page = region.page_size();
    // A misaligned superpage region would silently map the wrong span, so refuse
    // it here rather than rounding and hoping.
    assert_eq!(
        region.va % page,
        0,
        "region '{}' virtual base {:#x} is not aligned to its {page:#x}-byte page",
        region.name,
        region.va
    );
    assert_eq!(
        region.pa % page,
        0,
        "region '{}' physical base {:#x} is not aligned to its {page:#x}-byte page",
        region.name,
        region.pa
    );

    for index in 0..region.pages() {
        let offset = index * page;
        mapper.map_at_level(
            VirtualAddr::new(region.va + offset),
            PhysicalAddr::new(region.pa + offset),
            region.level,
            region.flags,
        )?;
    }
    Ok(())
}

/// Walk every page of `region` and require it to be exactly what was asked for.
///
/// Every page, not a sample: this runs once at boot and a wrong leaf anywhere is
/// either a fault or a silent protection hole.
fn verify(mapper: &KernelMapper<'_>, region: &Region) {
    let page = region.page_size();
    for index in 0..region.pages() {
        let offset = index * page;
        let va = VirtualAddr::new(region.va + offset);
        let (entry, level) = mapper
            .entry_of(va)
            .unwrap_or_else(|| panic!("region '{}' left {:#x} unmapped", region.name, va.bits()));

        assert_eq!(
            level, region.level,
            "region '{}' mapped {:#x} at level {level}, expected level {}",
            region.name, va.bits(), region.level
        );
        assert_eq!(
            entry.flags(),
            region.flags | PteFlags::VALID,
            "region '{}' has the wrong rights at {:#x}",
            region.name,
            va.bits()
        );
        assert_eq!(
            mapper.translate(va),
            Some(PhysicalAddr::new(region.pa + offset)),
            "region '{}' translates {:#x} to the wrong frame",
            region.name,
            va.bits()
        );
    }
}

/// Build the kernel's page table, verify it, and switch `satp` to it.
///
/// Call once, on the boot hart, after [`super::init`] — it allocates frames.
pub fn init() {
    let regions = regions();

    // W^X is checked on the layout itself, not merely intended by it. Cheap, and
    // it catches a bad flag constant before a single PTE is written.
    for region in &regions {
        let writable = region.flags.contains(PteFlags::WRITE);
        let executable = region.flags.contains(PteFlags::EXECUTE);
        assert!(
            !(writable && executable),
            "region '{}' is both writable and executable",
            region.name
        );
    }

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
        if region.len == 0 {
            continue;
        }
        install(&mut mapper, region)
            .unwrap_or_else(|error| panic!("mapping region '{}' failed: {error}", region.name));
    }

    for region in &regions {
        if region.len == 0 {
            continue;
        }
        verify(&mapper, region);
    }

    // The stack/heap guard page must really be a gap. The linker reserves it
    // explicitly (`_heap_start = _kernel_stack_end + 4096`), so a mapping here
    // would mean the region list had grown over a deliberate hole.
    let guard = layout::kernel_stack_end();
    assert!(
        mapper.entry_of(VirtualAddr::new(guard)).is_none(),
        "the stack/heap guard page at {guard:#x} is mapped; it must stay a hole to catch overruns"
    );

    // Finally the two addresses the switch itself depends on, read from the
    // *running* machine rather than assumed: if either is wrong we fault on the
    // instruction after `csrw satp`, with the old table already gone.
    let pc: usize;
    let sp: usize;
    // SAFETY: two reads of the current PC and stack pointer into locals.
    unsafe {
        core::arch::asm!("auipc {}, 0", out(reg) pc, options(nomem, nostack));
        core::arch::asm!("mv {}, sp", out(reg) sp, options(nomem, nostack));
    }
    check_live(&mapper, "instruction stream", pc, PteFlags::EXECUTE);
    check_live(&mapper, "stack pointer", sp, PteFlags::WRITE);

    log(root_pa, &regions);

    let satp = Satp::sv39(root_pa, 0);
    // SAFETY: `root` is a live Sv39 tree, just verified to map every kernel region
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

/// Page size as a count and a unit, derived from the level rather than restated.
fn page_label(level: usize) -> (usize, &'static str) {
    let size = page_size_at(level);
    if size >= GIGABYTE {
        (size / GIGABYTE, "GiB")
    } else if size >= MEGABYTE {
        (size / MEGABYTE, "MiB")
    } else {
        (size / KILOBYTE, "KiB")
    }
}

/// `rwx`-style rights, so the boot log states the protection policy plainly
/// instead of leaving it to be inferred from the source.
fn rights(flags: PteFlags) -> &'static str {
    match (
        flags.contains(PteFlags::READ),
        flags.contains(PteFlags::WRITE),
        flags.contains(PteFlags::EXECUTE),
    ) {
        (true, false, false) => "r--",
        (true, true, false) => "rw-",
        (true, false, true) => "r-x",
        (true, true, true) => "rwx",
        _ => "???",
    }
}

fn log(root_pa: PhysicalAddr, regions: &[Region]) {
    println!("[memory] kernel page table root at {:#x}:", root_pa.bits());
    for region in regions {
        if region.len == 0 {
            continue;
        }
        let (size, unit) = page_label(region.level);
        println!(
            "[memory]   {:<26} {:#018x} -> {:#012x}  {} {:>5} x {}{}",
            region.name,
            region.va,
            region.pa,
            rights(region.flags),
            region.pages(),
            size,
            unit
        );
    }
}
