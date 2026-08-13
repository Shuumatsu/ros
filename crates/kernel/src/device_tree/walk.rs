//! The single traversal: RAM extent, well-known devices, MMIO windows, carve-outs, hart
//! ids and the timebase, all from one `all_nodes()` pass.
//!
//! For correctness, not cost. A second traversal is a second *opinion* — two searches for
//! one device can settle on different nodes, and nothing downstream can tell.

use fdt_raw::{Fdt, Node};

use super::region::PhysRegion;
use super::table::{Device, DeviceTable, MAX_FOREIGN, MAX_HART_IDS, MAX_MMIO, Ram};

/// `compatible` strings for the devices the kernel resolves by name.
const UART: &[&str] = &["ns16550a", "ns16550"];
const PLIC: &[&str] = &["riscv,plic0", "sifive,plic-1.0.0"];
const CLINT: &[&str] = &["riscv,clint0", "sifive,clint0"];

/// What a node's `reg` property actually describes.
///
/// Mistaking one kind for another is a bug in both directions: reserved RAM read as a
/// device maps memory the firmware forbids, and a device read as RAM hands register space
/// to the allocator.
enum RegKind {
    /// A memory-mapped device window.
    Mmio,
    /// RAM carved out by the previous boot stage — present in memory, but not the
    /// kernel's to hand out.
    ReservedRam,
    /// `/cpus/cpu@N`: the `reg` is a **hart id**, not an address at all.
    HartId,
    /// The RAM itself.
    Ram,
}

/// Classify a node by its path — the single place this distinction is made.
///
/// `/reserved-memory/…` are firmware carve-outs, which OpenSBI adds for itself and its PMP
/// then denies to S-mode, so they must reach the frame allocator and never the page table.
/// They are absent from QEMU's own tree and appear only in the one OpenSBI hands on, so
/// `-machine dumpdtb` does not show them.
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

/// Whether the OS should ignore this node.
///
/// Absent `status` means enabled, per the spec; `disabled`, `fail`, `fail-sss` and
/// `reserved` all mean "not yours", so anything not explicitly fine is skipped.
///
/// Selection takes the first compatible node, so this is what stops a board listing an
/// unpopulated UART ahead of the real one from binding the console to a dead port — a
/// silent failure, since `console` drops the SBI fallback as soon as a base exists.
fn is_disabled(node: &Node<'_>) -> bool {
    !matches!(node.find_property_str("status"), None | Some("okay") | Some("ok"))
}

/// Read `interrupts`' first cell.
fn irq_of(node: &Node<'_>) -> Option<usize> {
    node.find_property("interrupts")?.as_u32_iter().next().map(|v| v as usize)
}

/// Read this node's first `reg` entry as a device window.
fn device_of(node: &Node<'_>) -> Option<Device> {
    let reg = node.reg()?.next()?;
    Some(Device {
        base: reg.address as usize,
        size: reg.size.unwrap_or(0) as usize,
        irq: irq_of(node),
    })
}

/// Record a foreign range, warning if the list is full rather than dropping it
/// quietly — an unrecorded carve-out is memory the allocator will hand out.
fn push_foreign(foreign: &mut heapless::Vec<PhysRegion, MAX_FOREIGN>, entry: PhysRegion) {
    if let Err(dropped) = foreign.push(entry) {
        println!(
            "[dtb] WARNING: more than {MAX_FOREIGN} foreign RAM ranges; {} at {:#x} is unreserved",
            dropped.name(),
            dropped.base
        );
    }
}

/// Walk the tree once and build the device table.
///
/// `kernel_pa` selects which `/memory` bank is ours: the one containing the kernel's
/// own physical load address, derived rather than hardcoded.
pub fn discover(fdt: &Fdt<'_>, blob: usize, blob_size: usize, kernel_pa: usize) -> DeviceTable {
    let mut mmio = heapless::Vec::new();
    let mut foreign = heapless::Vec::new();
    let mut hart_ids = heapless::Vec::new();
    let mut ram: Option<Ram> = None;
    let mut uart = None;
    let mut plic = None;
    let mut clint = None;
    let mut timebase_hz = None;
    let mut disabled = 0;

    // The blob is foreign RAM like any other: it sits in the pool and the allocator
    // would vend it.
    let blob_region = PhysRegion::new("device tree blob", blob, blob_size);
    push_foreign(&mut foreign, blob_region.clone());

    // The FDT header's reservation block: the spec's *other* mechanism, in the header
    // rather than the node tree, so reading it is not a second traversal.
    for (index, entry) in fdt.memory_reservations().enumerate() {
        if entry.size == 0 {
            continue;
        }
        let mut label: heapless::String<{ super::region::NAME_LEN }> = heapless::String::new();
        let _ = core::fmt::Write::write_fmt(&mut label, format_args!("fdt-rsvmap[{index}]"));
        push_foreign(
            &mut foreign,
            PhysRegion::new(&label, entry.address as usize, entry.size as usize),
        );
    }

    for node in fdt.all_nodes() {
        let name = node.name();
        let path = node.path();

        if is_disabled(&node) {
            disabled += 1;
            continue;
        }

        // `/chosen` and `/cpus` carry properties rather than a `reg`, so they are
        // read here rather than by a targeted lookup of their own.
        if &*path == "/chosen" {
            if let Some((start, end)) = initrd_range(&node) {
                push_foreign(&mut foreign, PhysRegion::new("initrd", start, end - start));
            }
            continue;
        }
        if &*path == "/cpus" {
            timebase_hz = node
                .find_property("timebase-frequency")
                .and_then(|p| p.as_u32().map(u64::from).or_else(|| p.as_u64()))
                .map(|hz| hz as usize);
            continue;
        }

        let kind = classify(name, &path);

        if matches!(kind, RegKind::Ram) {
            // Only the bank backing the kernel is ours to describe; others are reported
            // rather than dropped, since "one bank" and "the only bank" differ by however
            // much RAM the allocator never hears about.
            for reg in node.reg().into_iter().flatten() {
                let base = reg.address as usize;
                let end = base.saturating_add(reg.size.unwrap_or(0) as usize);
                if (base..end).contains(&kernel_pa) {
                    ram = Some(Ram { base, end });
                } else if end > base {
                    println!(
                        "[dtb] note: /memory bank {base:#x}..{end:#x} does not contain the \
                         kernel and is not managed"
                    );
                }
            }
            continue;
        }

        let Some(regs) = node.reg() else { continue };

        // A hart id is an address-shaped value with no size, so it is taken before
        // the size check that every real range must pass.
        if matches!(kind, RegKind::HartId) {
            for reg in regs {
                if hart_ids.push(reg.address as usize).is_err() {
                    println!("[dtb] WARNING: more than {MAX_HART_IDS} harts reported; ignoring rest");
                }
            }
            continue;
        }

        // Well-known devices resolve from the node we are already standing on, so
        // `reg` and `interrupts` cannot come from different nodes.
        let compatible = |list: &[&str]| node.compatibles().any(|c| list.contains(&c));
        if uart.is_none() && compatible(UART) {
            uart = device_of(&node);
        } else if plic.is_none() && compatible(PLIC) {
            plic = device_of(&node);
        } else if clint.is_none() && compatible(CLINT) {
            clint = device_of(&node);
        }

        // A node may describe several ranges — QEMU virt's `flash` has two.
        for reg in regs {
            // Loud, not a bare `continue`: a reserved range dropped in silence is memory
            // the allocator hands out. A missing size usually means `#size-cells = <0>`.
            let Some(size) = reg.size else {
                println!("[dtb] WARNING: {name} has a reg with no size; skipped");
                continue;
            };
            if size == 0 {
                println!("[dtb] WARNING: {name} has a zero-length reg at {:#x}; skipped", reg.address);
                continue;
            }
            let entry = PhysRegion::new(name, reg.address as usize, size as usize);

            // The two lists have different capacities, so they are different types
            // and cannot share one push site; only the diagnostic is shared.
            let overflowed = match kind {
                RegKind::Mmio => mmio.push(entry).err().map(|_| ("MMIO window", MAX_MMIO)),
                RegKind::ReservedRam => {
                    foreign.push(entry).err().map(|_| ("foreign RAM range", MAX_FOREIGN))
                }
                RegKind::HartId | RegKind::Ram => unreachable!("handled above"),
            };
            if let Some((what, cap)) = overflowed {
                println!(
                    "[dtb] WARNING: more than {cap} {what}s; {name} and any after it are \
                     unaccounted for"
                );
            }
        }
    }

    let ram = ram.unwrap_or_else(|| {
        panic!("[dtb] /memory has no region containing the kernel at {kernel_pa:#x}")
    });
    let uart = uart.expect("[dtb] no ns16550a UART node — cannot bring up the console");

    DeviceTable {
        blob: blob_region,
        ram,
        uart,
        plic,
        clint,
        timebase_hz,
        mmio,
        foreign,
        hart_ids,
        disabled,
    }
}

/// The initrd's extent, if the previous stage loaded one.
///
/// `fdt_raw`'s `Chosen` does not expose it, so the two properties are read directly.
/// They are `#address-cells`-sized, so 8 bytes here and 4 on a 32-bit tree.
fn initrd_range(chosen: &Node<'_>) -> Option<(usize, usize)> {
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
