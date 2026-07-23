//! Flattened Device Tree (FDT / DTB) discovery.
//!
//! The previous boot stage (QEMU with `-bios none`, or an SBI firmware) hands us
//! the physical address of the device tree blob in register `a1`. `boot.S`
//! preserves it, `start(dtb)` threads it through, and we parse it here with the
//! zero-allocation [`fdt_raw`] crate. No heap is required, so this is safe to run
//! before `memory::init` brings the allocator up.

use core::sync::atomic::{AtomicUsize, Ordering};

use fdt_raw::Fdt;

use crate::platform;

/// Physical address of the DTB, stashed at boot for later consumers.
static DTB_ADDR: AtomicUsize = AtomicUsize::new(0);

/// Returns the DTB physical address recorded at boot, or `None` if we never got one.
///
/// Kept for later consumers (drivers that want to re-parse the tree). Not yet
/// wired up, hence the allow.
#[allow(dead_code)]
pub fn dtb_addr() -> Option<usize> {
    match DTB_ADDR.load(Ordering::Relaxed) {
        0 => None,
        addr => Some(addr),
    }
}

/// Parse the device tree at `dtb_ptr` and dump what we found: memory regions,
/// boot arguments, and the primary UART.
///
/// # Safety
/// `dtb_ptr` must either be zero, or the address of a valid, readable FDT blob
/// (as passed in `a1`). The blob must stay mapped and unmodified for the duration
/// of this call — we borrow it in place (zero-copy) and drop the borrow on return.
pub unsafe fn init(dtb_ptr: usize) {
    if dtb_ptr == 0 {
        println!("[dtb] no device tree pointer in a1; skipping");
        return;
    }

    let fdt = match unsafe { Fdt::from_ptr(dtb_ptr as *mut u8) } {
        Ok(fdt) => fdt,
        Err(e) => {
            println!("[dtb] failed to parse FDT at {:#x}: {:?}", dtb_ptr, e);
            return;
        }
    };
    DTB_ADDR.store(dtb_ptr, Ordering::Relaxed);

    let header = fdt.header();
    println!(
        "[dtb] found at {:#x}: version {}, {} bytes, boot hart {}",
        dtb_ptr, header.version, header.totalsize, header.boot_cpuid_phys
    );

    // --- Physical memory ---
    for mem in fdt.memory() {
        for region in mem.regions() {
            let end = region.address.saturating_add(region.size);
            println!(
                "[dtb] memory: {:#x}..{:#x} ({} MiB)",
                region.address,
                end,
                region.size / (1024 * 1024)
            );
        }
    }

    // --- Boot parameters (/chosen) ---
    if let Some(chosen) = fdt.chosen() {
        if let Some(args) = chosen.bootargs() {
            println!("[dtb] bootargs: {:?}", args);
        }
        if let Some(path) = chosen.stdout_path() {
            println!("[dtb] stdout-path: {}", path);
        }
    }

    // --- Primary UART: cross-check the tree against our hardcoded constant ---
    for node in fdt.all_nodes() {
        if node.compatibles().any(|c| c == "ns16550a" || c == "ns16550") {
            if let Some(reg) = node.reg().and_then(|mut r| r.next()) {
                let base = reg.address as usize;
                let verdict = if base == platform::UART0_BASE { "OK" } else { "MISMATCH" };
                println!(
                    "[dtb] uart '{}' @ {:#x} (hardcoded UART0_BASE = {:#x}) => {}",
                    node.name(),
                    base,
                    platform::UART0_BASE,
                    verdict
                );
            }
            break;
        }
    }
}
