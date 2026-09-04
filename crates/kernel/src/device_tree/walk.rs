//! Single-pass discovery with bus ancestry and disabled-subtree tracking.

use fdt_raw::{Fdt, RegInfo};
use heapless::String;
use mmu::PhysicalAddr;

use super::bus::{self, Bridge, BusStack, PATH_LEN};
use super::console::{self, MAX_UARTS, UartNode};
use super::props::{self, RegKind};
use super::table::{Device, DeviceTable, MAX_HART_IDS};
use crate::drivers::uart16550;
use crate::memory::machine::{MAX_FOREIGN, MAX_MMIO};
use crate::memory::phys_range::{NAME_LEN, PhysRange};
use crate::utils::{ByteSize, truncated};

struct Bound {
    what: &'static str,
    lost: &'static str,
    limit: &'static str,
}

const MMIO_BOUND: Bound =
    Bound { what: "device windows", lost: "never be mapped", limit: "memory::machine::MAX_MMIO" };

const FOREIGN_BOUND: Bound = Bound {
    what: "foreign RAM ranges",
    lost: "go unreserved and be vended as free memory",
    limit: "memory::machine::MAX_FOREIGN",
};

fn push_bounded<const N: usize>(
    list: &mut heapless::Vec<PhysRange, N>,
    entry: PhysRange,
    bound: &Bound,
) {
    list.push(entry).unwrap_or_else(|dropped| {
        panic!(
            "[dtb] the machine describes more than {N} {}; '{}' at {:#x} would {} — raise {}",
            bound.what,
            dropped.name(),
            dropped.base,
            bound.lost,
            bound.limit
        )
    });
}

fn push_lossy<T, const N: usize>(
    list: &mut heapless::Vec<T, N>,
    entry: T,
    noted: &mut bool,
    note: impl FnOnce(),
) {
    if list.push(entry).is_err() && !core::mem::replace(noted, true) {
        note();
    }
}

fn resolve_range(name: &str, reg: &RegInfo, ancestors: &[Bridge<'_>]) -> Option<PhysRange> {
    let size = props::usable_size(name, reg)?;
    let Some(address) = bus::to_cpu_address(ancestors, reg.address) else {
        println!(
            "[dtb] WARNING: {name}'s reg at {:#x} is not an address any parent bus maps; skipped",
            reg.address
        );
        return None;
    };
    Some(PhysRange::new(name, props::phys(address), size))
}

/// Build a device table, selecting the RAM bank that contains `kernel_pa`.
pub fn discover<'a>(
    fdt: &Fdt<'a>,
    blob: PhysicalAddr,
    blob_size: usize,
    kernel_pa: PhysicalAddr,
) -> DeviceTable {
    let mut mmio: heapless::Vec<PhysRange, MAX_MMIO> = heapless::Vec::new();
    let mut foreign: heapless::Vec<PhysRange, MAX_FOREIGN> = heapless::Vec::new();
    let mut hart_ids: heapless::Vec<usize, MAX_HART_IDS> = heapless::Vec::new();
    let mut uarts: heapless::Vec<UartNode, MAX_UARTS> = heapless::Vec::new();
    let mut ram: Option<PhysRange> = None;
    let mut chosen_console: Option<String<PATH_LEN>> = None;
    let mut timebase_hz = None;
    let mut disabled = 0;
    let mut harts_dropped = false;
    let mut uarts_dropped = false;

    let mut stack = BusStack::new();
    let mut skipping: Option<String<PATH_LEN>> = None;

    // Reserve the blob itself from the frame allocator.
    let blob_region = PhysRange::new("device tree blob", blob, blob_size);
    push_bounded(&mut foreign, blob_region.clone(), &FOREIGN_BOUND);

    // Header reservation entries are independent of `/reserved-memory`.
    for (index, entry) in fdt.memory_reservations().enumerate() {
        if entry.size == 0 {
            continue;
        }
        let mut label: String<NAME_LEN> = String::new();
        let _ = core::fmt::Write::write_fmt(&mut label, format_args!("fdt-rsvmap[{index}]"));
        let entry = PhysRange::new(&label, props::phys(entry.address), entry.size as usize);
        push_bounded(&mut foreign, entry, &FOREIGN_BOUND);
    }

    for node in fdt.all_nodes() {
        let name = node.name();
        // Paths are unreliable beyond the parser's bounded depth.
        bus::require_depth(&node);
        let path = node.path();

        if let Some(prefix) = &skipping {
            if bus::is_below(prefix, &path) {
                disabled += 1;
                continue;
            }
            skipping = None;
        }
        if props::is_disabled(&node) {
            disabled += 1;
            skipping = Some(truncated(&path));
            continue;
        }

        let ancestors = stack.enter(&node, &path);

        if &*path == "/chosen" {
            if let Some((base, size)) = props::initrd_range(&node) {
                push_bounded(&mut foreign, PhysRange::new("initrd", base, size), &FOREIGN_BOUND);
            }
            chosen_console = node.find_property_str("stdout-path").map(console::console_path);
            continue;
        }
        if &*path == "/cpus" {
            timebase_hz = timebase_hz.or_else(|| props::timebase_of(&node));
            continue;
        }

        let Some(kind) = props::classify(&node, &path) else { continue };
        let Some(regs) = props::decoded_regs(&node, name) else { continue };

        match kind {
            // A hart `reg` is an identifier, not a bus address.
            RegKind::HartId => {
                timebase_hz = timebase_hz.or_else(|| props::timebase_of(&node));
                for reg in regs {
                    push_lossy(&mut hart_ids, reg.address as usize, &mut harts_dropped, || {
                        println!(
                            "[dtb] WARNING: machine reports more than the {MAX_HART_IDS} harts \
                             this kernel has cpu slots for; the rest are ignored"
                        )
                    });
                }
            }

            // This kernel manages only the bank containing its image.
            RegKind::Ram => {
                for reg in regs {
                    let Some(bank) = resolve_range(name, &reg, ancestors) else { continue };
                    if bank.contains(kernel_pa) {
                        ram = Some(bank);
                    } else {
                        println!(
                            "[dtb] WARNING: /memory bank {:#x}..{:#x} ({}) does not contain the \
                             kernel; this kernel manages one bank, so that RAM is lost",
                            bank.base,
                            bank.end(),
                            ByteSize(bank.size)
                        );
                    }
                }
            }

            RegKind::ReservedRam => {
                for reg in regs {
                    let Some(entry) = resolve_range(name, &reg, ancestors) else { continue };
                    push_bounded(&mut foreign, entry, &FOREIGN_BOUND);
                }
            }

            RegKind::Mmio => {
                let irq = props::irq_of(&node);
                let mut window = None;
                for reg in regs {
                    let Some(entry) = resolve_range(name, &reg, ancestors) else { continue };
                    let resolved = Device { base: entry.base, size: entry.size, irq };
                    push_bounded(&mut mmio, entry, &MMIO_BOUND);
                    window = window.or(Some(resolved));
                }

                if let Some(device) = window
                    && node.compatibles().any(|c| uart16550::COMPATIBLE.contains(&c))
                {
                    let candidate = UartNode { path: truncated(&path), device };
                    push_lossy(&mut uarts, candidate, &mut uarts_dropped, || {
                        println!(
                            "[dtb] note: more than {MAX_UARTS} UARTs; the rest are not console \
                             candidates"
                        )
                    });
                }
            }
        }
    }

    let ram = ram.unwrap_or_else(|| {
        panic!("[dtb] /memory has no region containing the kernel at {kernel_pa:#x}")
    });
    let uart = console::resolve(fdt, &uarts, chosen_console.as_deref())
        .expect("[dtb] no UART node this kernel has a driver for — no console is possible");

    DeviceTable { blob: blob_region, ram, uart, timebase_hz, mmio, foreign, hart_ids, disabled }
}
