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

// Discovered hardware, filled in by `discover`. Zero means "not found".
static DTB_ADDR: AtomicUsize = AtomicUsize::new(0);
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

    let fdt = match unsafe { Fdt::from_ptr(dtb_ptr as *mut u8) } {
        Ok(fdt) => fdt,
        Err(e) => panic!("[dtb] failed to parse FDT at {:#x}: {:?}", dtb_ptr, e),
    };
    DTB_ADDR.store(dtb_ptr, Ordering::Relaxed);

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
    println!("[dtb] blob at {:#x}", DTB_ADDR.load(Ordering::Relaxed));
    if let (Some(base), Some(end)) = (ram_base(), ram_end()) {
        println!("[dtb] ram:   {:#x}..{:#x} ({} MiB)", base, end, (end - base) / (1024 * 1024));
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
    }
}
