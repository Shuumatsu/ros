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

/// Physical extent of the device-tree blob itself, `[start, end)`.
///
/// The blob lives in ordinary RAM — on QEMU virt near the top of it — so it falls
/// inside the range the frame allocator manages. Something has to withhold it, or
/// the allocator will vend the memory the tree is still stored in;
/// [`crate::memory::frame::init`] reserves exactly this range.
///
/// `None` before the tree has been parsed.
pub fn dtb_range() -> Option<(usize, usize)> {
    let start = DTB_ADDR.load(Ordering::Relaxed);
    let size = DTB_SIZE.load(Ordering::Relaxed);
    if start == 0 || size == 0 {
        return None;
    }
    Some((start, start.saturating_add(size)))
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
const MMIO_NAME_LEN: usize = 40;

/// MMIO windows recordable. QEMU virt describes about sixteen.
const MAX_MMIO: usize = 48;

/// One MMIO window described by the device tree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MmioRegion {
    /// Device-tree node name, e.g. `serial@10000000`. Copied rather than borrowed
    /// so the list outlives the parse.
    name: String<MMIO_NAME_LEN>,
    /// Physical base of the window.
    pub base: usize,
    /// Window length in bytes.
    pub size: usize,
}

impl MmioRegion {
    /// The device-tree node name this window came from.
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Every MMIO window in the device tree, discovered by walking it.
static MMIO: Once<Vec<MmioRegion, MAX_MMIO>> = Once::new();

/// Every MMIO window the device tree describes.
///
/// The **single** answer to "where is device memory". Anything that needs to map,
/// protect or enumerate it reads the list from here rather than deciding on a range
/// of its own.
///
/// This is a genuine walk of the tree, not a fixed list of the devices this kernel
/// happens to drive. That distinction is the point: an earlier version returned only
/// UART, PLIC and CLINT while claiming to be complete, which left a future virtio
/// driver with no way to find its window here — and the path of least resistance
/// would have been to write its own base constant, recreating exactly the
/// split-brain this function exists to prevent.
///
/// A window appearing here says the *device* exists, not that supervisor mode may
/// touch it: OpenSBI's PMP configuration is a separate layer and denies S-mode
/// access to the CLINT on QEMU virt.
///
/// Empty before the tree has been parsed.
pub fn mmio_regions() -> &'static [MmioRegion] {
    MMIO.get().map(Vec::as_slice).unwrap_or(&[])
}

/// True if a node's `reg` describes something other than an MMIO window.
///
/// `reg` is not always a device address, and getting this wrong maps RAM as device
/// memory. Three kinds are excluded:
///
/// - `/memory@…` — the RAM itself, reported by [`ram_base`]/[`ram_end`] instead.
/// - `/cpus/cpu@N` — `reg` there is a hart id, not an address.
/// - `/reserved-memory/…` — RAM carved out by the previous boot stage. OpenSBI
///   *adds* these for its own firmware (`mmode_resv0`, `mmode_resv1` at the bottom
///   of RAM) and its PMP then denies supervisor access to them, so treating them as
///   devices would map memory the kernel must not touch. They are a frame-allocator
///   concern, not a device one.
///
/// Note the reserved-memory nodes do not appear in QEMU's own device tree — only in
/// the one OpenSBI hands on — so they are invisible to `-machine dumpdtb` and were
/// found by printing what the kernel actually walked.
fn reg_is_not_mmio(name: &str, path: &str) -> bool {
    name.starts_with("memory")
        || path.starts_with("/cpus")
        || path.starts_with("/reserved-memory")
}

/// Walk the tree and record every MMIO window.
fn discover_mmio(fdt: &Fdt<'_>) -> Vec<MmioRegion, MAX_MMIO> {
    let mut windows = Vec::new();

    for node in fdt.all_nodes() {
        let name = node.name();
        let path = node.path();
        if reg_is_not_mmio(name, &path) {
            continue;
        }
        let Some(regs) = node.reg() else { continue };

        // A node may describe several windows — QEMU virt's `flash` has two.
        for reg in regs {
            let Some(size) = reg.size else { continue };
            if size == 0 {
                continue;
            }
            let mut recorded = String::new();
            // Truncation only affects the label, never the address, so a long node
            // name must not drop the window.
            let _ = recorded.push_str(&name[..name.len().min(MMIO_NAME_LEN)]);
            let region =
                MmioRegion { name: recorded, base: reg.address as usize, size: size as usize };
            if windows.push(region).is_err() {
                println!(
                    "[dtb] WARNING: more than {MAX_MMIO} MMIO windows; {name} and any after \
                     it are unmapped"
                );
                return windows;
            }
        }
    }

    windows
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
    // the memory the tree is still living in. See `dtb_range`.
    DTB_SIZE.store(fdt.header().totalsize as usize, Ordering::Relaxed);

    // Walk the tree once for every MMIO window it describes. Done here, with the
    // blob borrowed, so nothing later needs to re-parse or guess.
    MMIO.call_once(|| discover_mmio(&fdt));

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
        println!("[dtb] ram:   {:#x}..{:#x} ({})", base, end, crate::utils::Bytes(end - base));
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
    let windows = mmio_regions();
    println!("[dtb] mmio:  {} windows discovered by walking the tree", windows.len());
    }
}
