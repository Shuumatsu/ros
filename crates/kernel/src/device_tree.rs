//! Flattened Device Tree (FDT / DTB) discovery.
//!
//! The previous boot stage (QEMU with `-bios none`, or an SBI firmware) hands us
//! the physical address of the device tree blob in register `a1`. `boot.S`
//! preserves it, `start(dtb)` threads it through, and we parse it here with the
//! zero-allocation [`fdt_raw`] crate. No heap is required, so discovery runs
//! before `memory::init` brings the allocator up — in fact before *anything*
//! prints, because the console itself learns the UART address from here.
//!
//! [`init`] parses the blob once and populates the device table (including the
//! UART base, which is what backs the console). [`summary`] prints the resolved
//! map from that stored state — no re-parse, so printing is decoupled from
//! discovery. Everything the rest of the kernel needs — RAM extent, UART
//! base/irq, PLIC and CLINT bases — comes from the accessors below, not from
//! hardcoded platform constants.

use core::sync::atomic::{AtomicUsize, Ordering};

use fdt_raw::Fdt;
use core::fmt::Write as _;

use heapless::{String, Vec};
use spin::Once;

// Discovered hardware, filled in by `discover`. Zero means "not found".
static DTB_ADDR: AtomicUsize = AtomicUsize::new(0);
static DTB_SIZE: AtomicUsize = AtomicUsize::new(0);
static RAM_BASE: AtomicUsize = AtomicUsize::new(0);
static RAM_END: AtomicUsize = AtomicUsize::new(0);
static UART_BASE: AtomicUsize = AtomicUsize::new(0);
static UART_SIZE: AtomicUsize = AtomicUsize::new(0);
static UART_IRQ: AtomicUsize = AtomicUsize::new(0);
static PLIC_BASE: AtomicUsize = AtomicUsize::new(0);
static PLIC_SIZE: AtomicUsize = AtomicUsize::new(0);
static CLINT_BASE: AtomicUsize = AtomicUsize::new(0);
static CLINT_SIZE: AtomicUsize = AtomicUsize::new(0);

// ============================================================================
// Accessors
// ============================================================================

/// DTB physical address recorded at boot, or `None` if we never got one.
#[allow(dead_code)]
pub fn dtb_addr() -> Option<usize> {
    match DTB_ADDR.load(Ordering::Relaxed) {
        0 => None,
        a => Some(a),
    }
}

/// Base of the RAM region backing the kernel, from the DTB.
#[allow(dead_code)]
pub fn ram_base() -> Option<usize> {
    match RAM_BASE.load(Ordering::Relaxed) {
        0 => None,
        a => Some(a),
    }
}

/// Exclusive end of the RAM region backing the kernel — the authoritative RAM
/// top. Prefer it over the linker's compile-time estimate.
pub fn ram_end() -> Option<usize> {
    match RAM_END.load(Ordering::Relaxed) {
        0 => None,
        a => Some(a),
    }
}

/// Primary UART base, or `None` before the device tree has been parsed. There is
/// no hardcoded UART address: the console falls back to the SBI console until
/// this is known, then uses the DTB-reported MMIO base.
pub fn uart_base() -> Option<usize> {
    match UART_BASE.load(Ordering::Relaxed) {
        0 => None,
        b => Some(b),
    }
}

/// Primary UART MMIO size (0 before discovery).
pub fn uart_size() -> usize {
    UART_SIZE.load(Ordering::Relaxed)
}

/// Primary UART interrupt number (0 before discovery).
pub fn uart_irq() -> usize {
    UART_IRQ.load(Ordering::Relaxed)
}

/// PLIC base. Panics if the tree carried no PLIC — callers only reach this once
/// external interrupts are being brought up, well after [`discover`].
pub fn plic_base() -> usize {
    require(&PLIC_BASE, "PLIC")
}

/// PLIC MMIO size (only meaningful alongside [`plic_base`]).
pub fn plic_size() -> usize {
    PLIC_SIZE.load(Ordering::Relaxed)
}

/// CLINT base. Panics if the tree carried no CLINT.
pub fn clint_base() -> usize {
    require(&CLINT_BASE, "CLINT")
}

/// CLINT MMIO size (only meaningful alongside [`clint_base`]).
pub fn clint_size() -> usize {
    CLINT_SIZE.load(Ordering::Relaxed)
}

/// Longest device-tree node name recorded. `virtio_mmio@10008000` is 20 characters.
const REGION_NAME_LEN: usize = 40;

/// MMIO windows recordable. QEMU virt describes seventeen.
const MAX_MMIO: usize = 48;

/// Foreign RAM ranges recordable: `/reserved-memory` nodes, FDT reservation-block
/// entries, the initrd and the blob itself.
const MAX_FOREIGN: usize = 24;

/// Hart ids recordable. Independent of how many the kernel has stacks for — the
/// machine reports what exists, `memory::stack` decides how many we can serve.
const MAX_HART_IDS: usize = 64;

/// A named physical address range taken from a device-tree node's `reg`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysRegion {
    /// Device-tree node name, e.g. `serial@10000000`. Copied rather than borrowed
    /// so the list outlives the parse.
    name: String<REGION_NAME_LEN>,
    /// Physical base of the range.
    pub base: usize,
    /// Length in bytes.
    pub size: usize,
}

impl PhysRegion {
    /// The device-tree node name this range came from.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Exclusive end of the range.
    pub fn end(&self) -> usize {
        self.base.saturating_add(self.size)
    }
}

/// What a node's `reg` property actually describes.
///
/// `reg` is not always a device address, and mistaking one kind for another is a
/// real bug in both directions: treating reserved RAM as a device maps memory the
/// firmware forbids, and treating a device as RAM would hand register space to the
/// allocator.
enum RegKind {
    /// A memory-mapped device window.
    Mmio,
    /// RAM carved out by the previous boot stage — present in memory, but not the
    /// kernel's to hand out.
    ReservedRam,
    /// `/cpus/cpu@N`: the `reg` is a **hart id**, not an address at all.
    HartId,
    /// The RAM itself, reported by [`ram_base`]/[`ram_end`] instead.
    Ram,
}

/// Classify a node by its path. The single place this distinction is made.
///
/// - `/memory@…` — the RAM itself, reported by [`ram_base`]/[`ram_end`] instead.
/// - `/cpus/cpu@N` — `reg` there is a hart id, not an address at all.
/// - `/reserved-memory/…` — firmware carve-outs. OpenSBI *adds* these for itself
///   (`mmode_resv0`/`mmode_resv1` at the bottom of RAM) and its PMP then denies
///   supervisor access, so they must reach the frame allocator, never the page table.
///
/// Note the reserved-memory nodes are absent from QEMU's own device tree and appear
/// only in the one OpenSBI hands on, so they are invisible to `-machine dumpdtb` —
/// they were found by printing what the kernel actually walked.
fn classify(name: &str, path: &str) -> RegKind {
    if path.starts_with("/reserved-memory") {
        RegKind::ReservedRam
    } else if path.starts_with("/cpus") && name.starts_with("cpu@") {
        RegKind::HartId
    } else if name.starts_with("memory") || path.starts_with("/cpus") {
        RegKind::Ram
    } else {
        RegKind::Mmio
    }
}

/// Everything one pass over the tree turns up.
struct Discovered {
    mmio: Vec<PhysRegion, MAX_MMIO>,
    foreign: Vec<PhysRegion, MAX_FOREIGN>,
    hart_ids: Vec<usize, MAX_HART_IDS>,
}

static DISCOVERED: Once<Discovered> = Once::new();

/// Every MMIO window the device tree describes.
///
/// The **single** answer to "where is device memory". Anything that needs to map,
/// protect or enumerate it reads the list from here rather than deciding on a range
/// of its own.
///
/// A genuine walk of the tree, not a fixed list of the devices this kernel happens to
/// drive. That distinction is the point: an earlier version returned only UART, PLIC
/// and CLINT while claiming to be complete, which left a future virtio driver with no
/// way to find its window here — and the path of least resistance would have been its
/// own base constant, recreating exactly the split-brain this exists to prevent.
///
/// A window appearing here says the *device* exists, not that supervisor mode may
/// touch it: OpenSBI's PMP is a separate layer and denies S-mode access to the CLINT
/// on QEMU virt.
///
/// Empty before the tree has been parsed.
pub fn mmio_regions() -> &'static [PhysRegion] {
    DISCOVERED.get().map(|d| d.mmio.as_slice()).unwrap_or(&[])
}

/// Every RAM range that exists but is not the kernel's to hand out.
///
/// The frame allocator would otherwise vend all of it. Four sources, because the
/// previous boot stage has four different ways of leaving something behind and
/// honouring only some of them is indistinguishable from honouring none:
///
/// 1. **`/reserved-memory` nodes** — firmware carve-outs. OpenSBI adds its own here
///    (`mmode_resv0`/`1`) and its PMP then denies supervisor access.
/// 2. **The FDT memory reservation block** (header `off_mem_rsvmap`) — the *other*,
///    older spec mechanism for exactly the same purpose. Empty on QEMU virt with
///    OpenSBI, but mandated by the spec and used by U-Boot and coreboot, so reading
///    only `/reserved-memory` honours half the standard.
/// 3. **The initrd**, from `/chosen`'s `linux,initrd-start` / `linux,initrd-end`.
///    QEMU puts a 32 MiB one at `0x84200000` — squarely inside the pool.
/// 4. **The blob itself**, which lives in ordinary RAM near the top of it.
///
/// Each entry is named by its source, so a later reclaim (an initrd is finished with
/// once the root filesystem is mounted) can find its range in the list.
///
/// Empty before the tree has been parsed.
pub fn foreign_ram() -> &'static [PhysRegion] {
    DISCOVERED.get().map(|d| d.foreign.as_slice()).unwrap_or(&[])
}

/// Every hart id the machine reports, from `/cpus/cpu@N`'s `reg`.
///
/// A list to iterate, never a range to count, and never a source of array indices:
/// these ids need not be `0..n`. See [`crate::memory::stack`] for why that matters
/// and what ignoring it cost.
///
/// Empty before the tree has been parsed.
pub fn hart_ids() -> &'static [usize] {
    DISCOVERED.get().map(|d| d.hart_ids.as_slice()).unwrap_or(&[])
}

/// Build a `PhysRegion`, truncating an over-long label but never the range.
fn region(name: &str, base: usize, size: usize) -> PhysRegion {
    let mut label = String::new();
    let _ = label.push_str(&name[..name.len().min(REGION_NAME_LEN)]);
    PhysRegion { name: label, base, size }
}

/// Record a foreign range, warning if the list is full rather than dropping it
/// quietly — an unrecorded carve-out is memory the allocator will hand out.
fn push_foreign(found: &mut Discovered, entry: PhysRegion) {
    if let Err(dropped) = found.foreign.push(entry) {
        println!(
            "[dtb] WARNING: more than {MAX_FOREIGN} foreign RAM ranges; {} at {:#x} is unreserved",
            dropped.name(),
            dropped.base
        );
    }
}

/// The initrd's extent, if the previous stage loaded one.
///
/// `fdt_raw`'s `Chosen` does not expose it, so the two properties are read directly.
/// They are `#address-cells`-sized, so 8 bytes here and 4 on a 32-bit tree.
fn initrd_range(fdt: &Fdt<'_>) -> Option<(usize, usize)> {
    let chosen = fdt.find_by_path("/chosen")?;
    let cell = |key: &str| {
        chosen
            .find_property(key)
            .and_then(|prop| prop.as_u64().or_else(|| prop.as_u32().map(u64::from)))
            .map(|value| value as usize)
    };
    let start = cell("linux,initrd-start")?;
    let end = cell("linux,initrd-end")?;
    (end > start).then_some((start, end))
}

/// Walk the tree once, recording device windows and firmware carve-outs.
///
/// One pass and one classifier for both, so the two lists cannot disagree about
/// which node is which.
fn discover_regions(fdt: &Fdt<'_>, blob: usize, blob_size: usize) -> Discovered {
    let mut found =
        Discovered { mmio: Vec::new(), foreign: Vec::new(), hart_ids: Vec::new() };

    // The blob is foreign RAM like any other: it sits in the pool and the allocator
    // would vend it.
    push_foreign(&mut found, region("device tree blob", blob, blob_size));

    // The FDT header's reservation block — the spec's *other* mechanism, separate
    // from the `/reserved-memory` nodes handled in the walk below.
    for (index, entry) in fdt.memory_reservations().enumerate() {
        if entry.size == 0 {
            continue;
        }
        let mut label = String::new();
        let _ = write!(label, "fdt-rsvmap[{index}]");
        push_foreign(
            &mut found,
            PhysRegion { name: label, base: entry.address as usize, size: entry.size as usize },
        );
    }

    // The initial ramdisk, which the previous stage loaded into RAM for us.
    if let Some((start, end)) = initrd_range(fdt) {
        push_foreign(&mut found, region("initrd", start, end - start));
    }

    for node in fdt.all_nodes() {
        let name = node.name();
        let path = node.path();
        let kind = classify(name, &path);
        if matches!(kind, RegKind::Ram) {
            continue;
        }
        let Some(regs) = node.reg() else { continue };

        // A hart id is an address-shaped value with no size, so it is taken before
        // the size check that every real range must pass.
        if matches!(kind, RegKind::HartId) {
            for reg in regs {
                if found.hart_ids.push(reg.address as usize).is_err() {
                    println!("[dtb] WARNING: more than {MAX_HART_IDS} harts reported; ignoring rest");
                }
            }
            continue;
        }

        // A node may describe several ranges — QEMU virt's `flash` has two.
        for reg in regs {
            let Some(size) = reg.size else { continue };
            if size == 0 {
                continue;
            }
            let entry = region(name, reg.address as usize, size as usize);

            // The two lists have different capacities, so they are different types
            // and cannot share one push site; only the diagnostic is shared.
            let overflowed = match kind {
                RegKind::Mmio => {
                    found.mmio.push(entry).err().map(|_| ("MMIO window", MAX_MMIO))
                }
                RegKind::ReservedRam => {
                    found.foreign.push(entry).err().map(|_| ("foreign RAM range", MAX_FOREIGN))
                }
                RegKind::HartId | RegKind::Ram => unreachable!("handled above"),
            };
            if let Some((what, cap)) = overflowed {
                // Dropping a reserved range silently would let the allocator vend
                // firmware memory, so this is loud rather than a debug aid.
                println!(
                    "[dtb] WARNING: more than {cap} {what}s; {name} and any after it are \
                     unaccounted for"
                );
            }
        }
    }

    found
}

fn require(cell: &AtomicUsize, what: &str) -> usize {
    match cell.load(Ordering::Relaxed) {
        0 => panic!("[dtb] {what} base not discovered (device_tree::discover not run, or device absent)"),
        v => v,
    }
}

// ============================================================================
// Discovery
// ============================================================================

/// First `reg` (address, size) of the first node whose `compatible` list
/// intersects `compatibles`.
fn find_reg(fdt: &Fdt, compatibles: &[&str]) -> Option<(usize, usize)> {
    for node in fdt.all_nodes() {
        if node.compatibles().any(|c| compatibles.contains(&c)) {
            if let Some(reg) = node.reg().and_then(|mut r| r.next()) {
                return Some((reg.address as usize, reg.size.unwrap_or(0) as usize));
            }
        }
    }
    None
}

/// First `interrupts` cell of the first node whose `compatible` list intersects
/// `compatibles`.
fn find_irq(fdt: &Fdt, compatibles: &[&str]) -> Option<usize> {
    for node in fdt.all_nodes() {
        if node.compatibles().any(|c| compatibles.contains(&c)) {
            if let Some(prop) = node.find_property("interrupts") {
                return prop.as_u32_iter().next().map(|v| v as usize);
            }
        }
    }
    None
}

/// Parse the device tree at `dtb_ptr` and record the hardware the kernel needs
/// (RAM extent, UART, PLIC, CLINT). Populating the UART base here is what backs
/// the console, so this must run before the first print — but `init` does not
/// print itself; call [`summary`] for that.
///
/// A usable device tree is part of the boot contract: a null pointer, an
/// unparseable blob, a `/memory` with no region containing the kernel, or a
/// missing UART all **panic** rather than let the kernel limp on wrong
/// addresses. (Such a panic is still visible via the SBI console.)
///
/// # Safety
/// `dtb_ptr` must be the address of a valid, readable FDT blob (as passed in
/// `a1`), and the blob must stay mapped and unmodified — we borrow it in place.
pub unsafe fn init(dtb_ptr: usize) {
    if dtb_ptr == 0 {
        panic!("[dtb] no device tree pointer in a1 — previous boot stage violated the boot contract");
    }

    // `a1` holds a *physical* address; reach the blob through the direct map
    // rather than treating it as a pointer. Both the boot table and the kernel
    // table map it there, whereas the raw address is only dereferenceable while a
    // boot identity mapping exists — and it no longer does.
    let blob = crate::memory::phys_to_virt(dtb_ptr) as *mut u8;
    // SAFETY: forwarded from this function's contract — `dtb_ptr` addresses a
    // valid FDT that stays mapped and unmodified, and `blob` is its direct-map
    // alias.
    let fdt = match unsafe { Fdt::from_ptr(blob) } {
        Ok(fdt) => fdt,
        Err(e) => panic!("[dtb] failed to parse FDT at {:#x}: {:?}", dtb_ptr, e),
    };
    DTB_ADDR.store(dtb_ptr, Ordering::Relaxed);
    // The blob's own extent, from its header. Needed because the blob sits *in*
    // RAM: the frame allocator has to be told to withhold it, or it will hand out
    // the memory the tree is still living in; see `foreign_ram`.
    DTB_SIZE.store(fdt.header().totalsize as usize, Ordering::Relaxed);

    // Walk the tree once for every MMIO window it describes. Done here, with the
    // blob borrowed, so nothing later needs to re-parse or guess.
    DISCOVERED.call_once(|| discover_regions(&fdt, dtb_ptr, fdt.header().totalsize as usize));

    // ---- Populate the device table (no printing yet: the console needs the
    //      UART base we are about to store). ----

    // Physical RAM: pick the /memory region that actually backs the kernel — the
    // one containing our own physical load address (derived, not hardcoded).
    let kernel_pa = crate::memory::virt_to_phys(crate::memory::layout::text_start());
    let mut ram_found = false;
    for mem in fdt.memory() {
        for region in mem.regions() {
            let base = region.address as usize;
            let end = base.saturating_add(region.size as usize);
            if (base..end).contains(&kernel_pa) {
                RAM_BASE.store(base, Ordering::Relaxed);
                RAM_END.store(end, Ordering::Relaxed);
                ram_found = true;
            }
        }
    }
    if !ram_found {
        panic!("[dtb] /memory has no region containing the kernel at {:#x}", kernel_pa);
    }

    // Primary UART — console-critical, so its absence is fatal.
    match find_reg(&fdt, &["ns16550a", "ns16550"]) {
        Some((base, size)) => {
            UART_BASE.store(base, Ordering::Relaxed);
            UART_SIZE.store(size, Ordering::Relaxed);
        }
        None => panic!("[dtb] no ns16550a UART node — cannot bring up the console"),
    }
    if let Some(irq) = find_irq(&fdt, &["ns16550a", "ns16550"]) {
        UART_IRQ.store(irq, Ordering::Relaxed);
    }

    // Interrupt controllers — optional; dormant until we enable interrupts.
    if let Some((base, size)) = find_reg(&fdt, &["riscv,plic0", "sifive,plic-1.0.0"]) {
        PLIC_BASE.store(base, Ordering::Relaxed);
        PLIC_SIZE.store(size, Ordering::Relaxed);
    }
    if let Some((base, size)) = find_reg(&fdt, &["riscv,clint0", "sifive,clint0"]) {
        CLINT_BASE.store(base, Ordering::Relaxed);
        CLINT_SIZE.store(size, Ordering::Relaxed);
    }
}

/// Print the resolved device map. Reads only the stored table — no re-parse —
/// so printing is fully decoupled from [`init`]. Call once the console is up,
/// i.e. after `init` (which is what backs the console with the real UART).
pub fn summary() {
    // Size included so the frame reservation in `memory::frame` can be checked
    // against it straight from the boot log.
    println!(
        "[dtb] blob at {:#x} (size {:#x})",
        DTB_ADDR.load(Ordering::Relaxed),
        DTB_SIZE.load(Ordering::Relaxed)
    );
    if let (Some(base), Some(end)) = (ram_base(), ram_end()) {
        println!("[dtb] ram:   {:#x}..{:#x} ({})", base, end, crate::utils::ByteSize(end - base));
    }
    println!(
        "[dtb] uart:  {:#x} (size {:#x}, irq {})",
        UART_BASE.load(Ordering::Relaxed),
        uart_size(),
        uart_irq()
    );
    if PLIC_BASE.load(Ordering::Relaxed) != 0 {
        println!("[dtb] plic:  {:#x} (size {:#x})", plic_base(), plic_size());
    }
    if CLINT_BASE.load(Ordering::Relaxed) != 0 {
        println!("[dtb] clint: {:#x} (size {:#x})", clint_base(), clint_size());
    println!(
        "[dtb] mmio:  {} windows, {} foreign RAM ranges (from one pass over the tree)",
        mmio_regions().len(),
        foreign_ram().len()
    );
    println!("[dtb] harts: {:?} (ids as reported, not a count)", hart_ids());
    }
}
