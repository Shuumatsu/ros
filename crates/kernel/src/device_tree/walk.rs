//! Single-pass discovery with bus ancestry and disabled-subtree tracking.

use fdt_raw::{Fdt, Node, RegInfo, RegIter};
use heapless::String;
use mmu::PhysicalAddr;

use super::PATH_LEN;
use super::bus::{self, Bridge, BusStack};
use super::console::{self, MAX_UARTS, UartNode};
use super::props::{self, RegKind};
use super::table::{Device, DeviceTable};
use crate::cpu::MAX_CPUS;
use crate::drivers::uart16550;
use crate::memory::machine::{MAX_FOREIGN, MAX_MMIO};
use crate::memory::phys_range::PhysRange;
use crate::utils::{ByteSize, truncated};

fn resolve_range(name: &str, reg: &RegInfo, ancestors: &[Bridge<'_>]) -> Option<PhysRange> {
    let size = props::usable_size(name, reg)?;
    let Some(address) = bus::to_cpu_address(ancestors, reg.address) else {
        println!(
            "[dtb] WARNING: {name}'s reg at {:#x} is not an address any parent bus maps; skipped",
            reg.address
        );
        return None;
    };
    Some(PhysRange::new(name, PhysicalAddr::new(address as usize), size))
}

/// What one traversal has found so far.
///
/// Every bounded list keeps the consequence of overflowing it: a lost device window or RAM
/// reservation is fatal, and a lost spare hart or console candidate is reported and dropped.
struct Discovery<'a> {
    mmio: heapless::Vec<PhysRange, MAX_MMIO>,
    foreign: heapless::Vec<PhysRange, MAX_FOREIGN>,
    hart_ids: heapless::Vec<usize, MAX_CPUS>,
    uarts: heapless::Vec<UartNode, MAX_UARTS>,
    ram: Option<PhysRange>,
    stdout: Option<String<PATH_LEN>>,
    aliases: Option<Node<'a>>,
    timebase_hz: Option<u64>,
    disabled: usize,
    harts_dropped: bool,
    uarts_dropped: bool,
    kernel_pa: PhysicalAddr,
}

impl<'a> Discovery<'a> {
    fn new(kernel_pa: PhysicalAddr) -> Self {
        Self {
            mmio: heapless::Vec::new(),
            foreign: heapless::Vec::new(),
            hart_ids: heapless::Vec::new(),
            uarts: heapless::Vec::new(),
            ram: None,
            stdout: None,
            aliases: None,
            timebase_hz: None,
            disabled: 0,
            harts_dropped: false,
            uarts_dropped: false,
            kernel_pa,
        }
    }

    fn add_window(&mut self, entry: PhysRange) {
        self.mmio.push(entry).unwrap_or_else(|dropped| {
            panic!(
                "[dtb] the machine describes more than {MAX_MMIO} device windows; '{}' at {:#x} \
                 would never be mapped — raise memory::machine::MAX_MMIO",
                dropped.name(),
                dropped.base
            )
        })
    }

    fn reserve(&mut self, entry: PhysRange) {
        self.foreign.push(entry).unwrap_or_else(|dropped| {
            panic!(
                "[dtb] the machine describes more than {MAX_FOREIGN} foreign RAM ranges; '{}' at \
                 {:#x} would go unreserved and be vended as free memory — raise \
                 memory::machine::MAX_FOREIGN",
                dropped.name(),
                dropped.base
            )
        })
    }

    fn add_hart(&mut self, id: usize) {
        if self.hart_ids.push(id).is_err() && !core::mem::replace(&mut self.harts_dropped, true) {
            println!(
                "[dtb] WARNING: the machine reports more than the {MAX_CPUS} harts this kernel \
                 has cpu slots for; the rest are ignored"
            );
        }
    }

    fn add_uart(&mut self, uart: UartNode) {
        if self.uarts.push(uart).is_err() && !core::mem::replace(&mut self.uarts_dropped, true) {
            println!(
                "[dtb] WARNING: the machine reports more than {MAX_UARTS} UARTs; the rest are \
                 not console candidates"
            );
        }
    }

    /// A hart `reg` is an identifier, not a bus address.
    fn harts(&mut self, node: &Node<'_>, regs: RegIter<'_>) {
        self.timebase_hz = self.timebase_hz.or_else(|| props::timebase_of(node));
        for reg in regs {
            self.add_hart(reg.address as usize);
        }
    }

    /// This kernel manages only the bank containing its image.
    fn memory(&mut self, name: &str, regs: RegIter<'_>, ancestors: &[Bridge<'_>]) {
        for reg in regs {
            let Some(bank) = resolve_range(name, &reg, ancestors) else { continue };
            if bank.contains(self.kernel_pa) {
                self.ram = Some(bank);
            } else {
                println!(
                    "[dtb] WARNING: /memory bank {:#x}..{:#x} ({}) does not contain the kernel; \
                     this kernel manages one bank, so that RAM is lost",
                    bank.base,
                    bank.end(),
                    ByteSize(bank.size)
                );
            }
        }
    }

    fn reserved(&mut self, name: &str, regs: RegIter<'_>, ancestors: &[Bridge<'_>]) {
        for reg in regs {
            let Some(entry) = resolve_range(name, &reg, ancestors) else { continue };
            self.reserve(entry);
        }
    }

    /// Record every window the node publishes, and keep the first as the device's own.
    fn device(&mut self, node: &Node<'_>, path: &str, regs: RegIter<'_>, ancestors: &[Bridge<'_>]) {
        let irq = props::irq_of(node);
        let mut window = None;
        for reg in regs {
            let Some(entry) = resolve_range(node.name(), &reg, ancestors) else { continue };
            window.get_or_insert(Device { base: entry.base, size: entry.size, irq });
            self.add_window(entry);
        }

        if let Some(device) = window
            && node.compatibles().any(|c| uart16550::COMPATIBLE.contains(&c))
        {
            self.add_uart(UartNode { path: truncated(path), device });
        }
    }

    fn finish(self, blob: PhysRange) -> DeviceTable {
        let kernel_pa = self.kernel_pa;
        let ram = self.ram.unwrap_or_else(|| {
            panic!("[dtb] /memory has no region containing the kernel at {kernel_pa:#x}")
        });
        let uart = console::resolve(&self.uarts, self.stdout.as_deref(), self.aliases.as_ref())
            .expect("[dtb] no UART node this kernel has a driver for — no console is possible");

        DeviceTable {
            blob,
            ram,
            uart,
            timebase_hz: self.timebase_hz,
            mmio: self.mmio,
            foreign: self.foreign,
            hart_ids: self.hart_ids,
            disabled: self.disabled,
        }
    }
}

/// Build a device table, selecting the RAM bank that contains `kernel_pa`.
pub fn discover<'a>(
    fdt: &Fdt<'a>,
    blob: PhysicalAddr,
    blob_size: usize,
    kernel_pa: PhysicalAddr,
) -> DeviceTable {
    let mut found = Discovery::new(kernel_pa);
    let mut stack = BusStack::new();
    let mut skipping: Option<usize> = None;

    // Reserve the blob itself from the frame allocator.
    let blob_region = PhysRange::new("device tree blob", blob, blob_size);
    found.reserve(blob_region.clone());

    // Header reservation entries are independent of `/reserved-memory`.
    for (index, entry) in fdt.memory_reservations().enumerate() {
        if entry.size == 0 {
            continue;
        }
        found.reserve(PhysRange::labeled(
            format_args!("fdt-rsvmap[{index}]"),
            PhysicalAddr::new(entry.address as usize),
            entry.size as usize,
        ));
    }

    for node in fdt.all_nodes() {
        let path = bus::path_of(&node);

        if let Some(level) = skipping {
            if node.level() > level {
                found.disabled += 1;
                continue;
            }
            skipping = None;
        }
        if props::is_disabled(&node) {
            found.disabled += 1;
            skipping = Some(node.level());
            continue;
        }

        let ancestors = stack.enter(&node);

        match &*path {
            "/chosen" => {
                if let Some((base, size)) = props::initrd_range(&node) {
                    found.reserve(PhysRange::new("initrd", base, size));
                }
                found.stdout = node.find_property_str("stdout-path").map(console::console_path);
                continue;
            }
            "/aliases" => {
                found.aliases = Some(node);
                continue;
            }
            "/cpus" => {
                found.timebase_hz = found.timebase_hz.or_else(|| props::timebase_of(&node));
                continue;
            }
            _ => {}
        }

        let Some(kind) = props::classify(&node, &path) else { continue };
        let Some(regs) = props::decoded_regs(&node) else { continue };

        match kind {
            RegKind::HartId => found.harts(&node, regs),
            RegKind::Ram => found.memory(node.name(), regs, ancestors),
            RegKind::ReservedRam => found.reserved(node.name(), regs, ancestors),
            RegKind::Mmio => found.device(&node, &path, regs, ancestors),
        }
    }

    found.finish(blob_region)
}
