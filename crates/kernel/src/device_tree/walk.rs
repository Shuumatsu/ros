//! The single traversal: RAM extent, well-known devices, MMIO windows, carve-outs, hart
//! ids and the timebase, all from one `all_nodes()` pass.
//!
//! For correctness, not cost. A second traversal is a second *opinion* — two searches for
//! one device can settle on different nodes, and nothing downstream can tell.
//!
//! The pass carries two pieces of state a flat iterator does not: the stack of parent
//! buses, so a `reg` is read in the address space the CPU sees rather than the one its bus
//! publishes, and the path of a disabled node, so its whole subtree goes with it.

use fdt_raw::{Fdt, Node, Property, RegInfo};
use heapless::String;
use paging::PhysicalAddr;

use super::table::{Device, DeviceTable, MAX_HART_IDS};
use crate::memory::machine::{MAX_FOREIGN, MAX_MMIO, NAME_LEN, PhysRange};
use crate::utils::truncated;

/// `compatible` strings for the devices the kernel resolves by name.
const UART: &[&str] = &["ns16550a", "ns16550"];
const PLIC: &[&str] = &["riscv,plic0", "sifive,plic-1.0.0"];
const CLINT: &[&str] = &["riscv,clint0", "sifive,clint0"];

/// Longest node path kept, for the bus stack and the console lookup. Paths are compared
/// by prefix, so a tree with one longer than this would have to be pathological before the
/// truncation could confuse two nodes — `/soc/virtio_mmio@10008000` is 26 characters.
const PATH_LEN: usize = 128;

/// Parent buses tracked at once, which is the tree's depth rather than its width.
const MAX_DEPTH: usize = 16;

/// UART nodes remembered while looking for the one `/chosen` names.
const MAX_UARTS: usize = 4;

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
    /// A hart: the `reg` is a **hart id**, not an address at all.
    HartId,
    /// The RAM itself.
    Ram,
    /// A node under `/cpus` that is not a hart. A `reg` there is neither RAM nor a window
    /// the kernel maps, so it is left alone rather than guessed at.
    Ignored,
}

/// Classify a node by what its `reg` describes — the single place this distinction is made.
///
/// `device_type` is the spec's discriminator for the two kinds of node whose `reg` is not a
/// device window, and QEMU, U-Boot and OpenSBI all emit it. Anything else carrying a `reg`
/// is a device.
///
/// `/reserved-memory/…` are firmware carve-outs, which OpenSBI adds for itself and its PMP
/// then denies to S-mode, so they must reach the frame allocator. They are absent from
/// QEMU's own tree and appear only in the one OpenSBI hands on, so `-machine dumpdtb` does
/// not show them.
fn classify(node: &Node<'_>, path: &str) -> RegKind {
    if path.starts_with("/reserved-memory/") {
        return RegKind::ReservedRam;
    }
    match node.find_property_str("device_type") {
        Some("memory") => RegKind::Ram,
        Some("cpu") => RegKind::HartId,
        _ if path.starts_with("/cpus/") => RegKind::Ignored,
        _ => RegKind::Mmio,
    }
}

/// Whether the OS should ignore this node and everything under it.
///
/// Absent `status` means enabled, per the spec; `disabled`, `fail`, `fail-sss` and
/// `reserved` all mean "not yours", so anything not explicitly fine is skipped.
fn is_disabled(node: &Node<'_>) -> bool {
    !matches!(node.find_property_str("status"), None | Some("okay") | Some("ok"))
}

/// Whether `path` names a node strictly below `ancestor`.
///
/// The component boundary matters: `/soc-foo` is not below `/soc`. The root is never an
/// ancestor by this test, which is what it should be — its children's `reg` are already
/// CPU addresses, so there is nothing to translate through.
fn is_below(ancestor: &str, path: &str) -> bool {
    path.len() > ancestor.len()
        && path.starts_with(ancestor)
        && path.as_bytes()[ancestor.len()] == b'/'
}

/// Take `count` big-endian cells as one number, or `None` if the property runs out.
fn take_cells(cells: &mut impl Iterator<Item = u32>, count: usize) -> Option<u64> {
    (0..count).try_fold(0u64, |value, _| Some((value << 32) | u64::from(cells.next()?)))
}

/// A parent bus, and how it maps its children's addresses into its own space.
///
/// The cell counts are carried rather than read back off the node because one `ranges`
/// entry is laid out by three of them, from two different nodes: a child address in *this*
/// bus's `#address-cells`, a parent address in its parent's, and a length in this bus's
/// `#size-cells`. The walk knows all three because it holds the chain; a node on its own
/// does not.
struct Bridge<'a> {
    path: String<PATH_LEN>,
    /// The node's `ranges`. `None` means the property is absent, which per the spec means
    /// the child address space is *not* mapped into the parent's at all — a very different
    /// thing from an empty `ranges`, which means it maps one-to-one.
    ranges: Option<Property<'a>>,
    /// Cells of a child address on this bus, which is also its `#address-cells`.
    child_cells: usize,
    /// Cells of a parent address, i.e. the parent bus's `#address-cells`.
    parent_cells: usize,
    /// Cells of a length on this bus.
    size_cells: usize,
}

impl Bridge<'_> {
    /// Carry one child-bus address into this bus's own space, or `None` if this bus does
    /// not map it.
    fn translate(&self, address: u64) -> Option<u64> {
        let ranges = self.ranges.as_ref()?;
        let mut cells = ranges.as_u32_iter();
        let mut entries = 0;

        while let Some(child) = take_cells(&mut cells, self.child_cells) {
            let (Some(parent), Some(length)) = (
                take_cells(&mut cells, self.parent_cells),
                take_cells(&mut cells, self.size_cells),
            ) else {
                // A truncated entry means the cell counts and the property disagree, so
                // every entry read so far is suspect too.
                return None;
            };
            entries += 1;
            if let Some(offset) = address.checked_sub(child)
                && offset < length
            {
                return Some(parent.wrapping_add(offset));
            }
        }

        // An empty `ranges` maps its children one-to-one. A non-empty one that matched
        // nothing does not map this address at all, and saying so is the point: the
        // untranslated value would name some unrelated physical page.
        (entries == 0).then_some(address)
    }
}

/// Carry a child-bus address all the way up to the address the CPU issues.
///
/// A `reg` is written in the address space its parent bus publishes, and only an identity
/// `ranges` at every level makes that the CPU's. Recording an untranslated address instead
/// would map some unrelated physical page and call it a device.
fn to_cpu_address(ancestors: &[Bridge<'_>], address: u64) -> Option<u64> {
    ancestors.iter().rev().try_fold(address, |address, bridge| bridge.translate(address))
}

/// A `reg` entry's length, or `None` with a diagnostic.
///
/// Loud in every case, and shared by every kind of range: a device with no window is a
/// driver that cannot bind, and a carve-out dropped in silence is memory the allocator
/// hands out. A missing size usually means `#size-cells = <0>`.
fn usable_size(name: &str, reg: &RegInfo) -> Option<usize> {
    match reg.size {
        None => {
            println!("[dtb] WARNING: {name} has a reg at {:#x} with no size; skipped", reg.address);
            None
        }
        Some(0) => {
            println!("[dtb] WARNING: {name} has a zero-length reg at {:#x}; skipped", reg.address);
            None
        }
        Some(size) => Some(size as usize),
    }
}

/// The interrupt a node raises, when the tree states it unambiguously.
///
/// One cell and one only. The number of cells is the *controller's* to declare through
/// `#interrupt-cells`, and that node is reached by phandle rather than from here; under a
/// two- or three-cell encoding the first cell is a type or a flags word, not a number the
/// kernel could program. `interrupts-extended` is likewise absent, since it names a
/// controller per entry. Reporting nothing beats reporting a number that means something
/// else.
fn irq_of(node: &Node<'_>) -> Option<usize> {
    let property = node.find_property("interrupts")?;
    let mut cells = property.as_u32_iter();
    let first = cells.next()?;
    cells.next().is_none().then_some(first as usize)
}

/// `timebase-frequency`, which the spec allows on `/cpus` or on any hart below it.
fn timebase_of(node: &Node<'_>) -> Option<usize> {
    node.find_property("timebase-frequency")
        .and_then(|property| property.as_u32().map(u64::from).or_else(|| property.as_u64()))
        .map(|hz| hz as usize)
}

/// Record a foreign range, warning if the list is full rather than dropping it
/// quietly — an unrecorded carve-out is memory the allocator will hand out.
///
/// The one way into that list, so every source of one is bounded and reported alike.
fn push_foreign(foreign: &mut heapless::Vec<PhysRange, MAX_FOREIGN>, entry: PhysRange) {
    if let Err(dropped) = foreign.push(entry) {
        println!(
            "[dtb] WARNING: more than {MAX_FOREIGN} foreign RAM ranges; {} at {:#x} is unreserved",
            dropped.name(),
            dropped.base
        );
    }
}

/// A `reg` cell as a physical address. The tree reports raw integers; this is the one place
/// they become addresses, so nothing downstream has to decide what they are.
fn phys(address: u64) -> PhysicalAddr { PhysicalAddr::new(address as usize) }

/// A UART the kernel could drive, kept until `/chosen` has been read and the console can
/// be chosen rather than guessed.
struct UartNode {
    path: String<PATH_LEN>,
    device: Device,
}

/// Walk the tree once and build the device table.
///
/// `kernel_pa` selects which `/memory` bank is ours: the one containing the kernel's
/// own physical load address, derived rather than hardcoded.
pub fn discover<'a>(
    fdt: &Fdt<'a>,
    blob: PhysicalAddr,
    blob_size: usize,
    kernel_pa: PhysicalAddr,
) -> DeviceTable {
    let mut mmio = heapless::Vec::new();
    let mut foreign = heapless::Vec::new();
    let mut hart_ids: heapless::Vec<usize, MAX_HART_IDS> = heapless::Vec::new();
    let mut ram: Option<PhysRange> = None;
    let mut uarts: heapless::Vec<UartNode, MAX_UARTS> = heapless::Vec::new();
    let mut console: Option<String<PATH_LEN>> = None;
    let mut plic = None;
    let mut clint = None;
    let mut timebase_hz = None;
    let mut disabled = 0;
    let mut harts_dropped = false;

    // The buses above the node being visited, nearest last. Depth-first order is what lets
    // one stack stand in for a parent pointer the flat iterator does not give us.
    let mut bridges: heapless::Vec<Bridge<'a>, MAX_DEPTH> = heapless::Vec::new();
    // The spec's default until `/` says otherwise, which it does before any other node.
    let mut root_address_cells = 2;
    // The disabled node whose subtree is being skipped, if any.
    let mut skipping: Option<String<PATH_LEN>> = None;

    // The blob is foreign RAM like any other: it sits in the pool and the allocator
    // would vend it.
    let blob_region = PhysRange::new("device tree blob", blob, blob_size);
    push_foreign(&mut foreign, blob_region.clone());

    // The FDT header's reservation block: the spec's *other* mechanism, in the header
    // rather than the node tree, so reading it is not a second traversal.
    for (index, entry) in fdt.memory_reservations().enumerate() {
        if entry.size == 0 {
            continue;
        }
        let mut label: String<NAME_LEN> = String::new();
        let _ = core::fmt::Write::write_fmt(&mut label, format_args!("fdt-rsvmap[{index}]"));
        push_foreign(
            &mut foreign,
            PhysRange::new(&label, phys(entry.address), entry.size as usize),
        );
    }

    for node in fdt.all_nodes() {
        let name = node.name();
        let path = node.path();

        // Everything below a disabled node is disabled with it: a bus the firmware says is
        // not ours does not become ours one child at a time.
        if let Some(prefix) = &skipping {
            if is_below(prefix, &path) {
                disabled += 1;
                continue;
            }
            skipping = None;
        }
        if is_disabled(&node) {
            disabled += 1;
            skipping = Some(truncated(&path));
            continue;
        }

        // Parent buses first, so `ancestors` below is exactly the chain this node's `reg`
        // has to climb. This node goes on too, for whichever of its children comes next.
        while bridges.last().is_some_and(|bridge| !is_below(&bridge.path, &path)) {
            bridges.pop();
        }
        // `is_below` never keeps the root, so it is never on the stack when its children
        // are visited — and its `#address-cells` is exactly what a top-level node's
        // `ranges` counts a parent address in. Hence the separate copy.
        if &*path == "/" {
            root_address_cells = node.address_cells as usize;
        }
        let bridge = Bridge {
            path: truncated(&path),
            ranges: node.find_property("ranges"),
            child_cells: node.address_cells as usize,
            parent_cells: bridges.last().map_or(root_address_cells, |parent| parent.child_cells),
            size_cells: node.size_cells as usize,
        };
        // Everything but this node, so a `reg` is not translated through its own bus. A
        // node too deep to push is not on the stack, so nothing is dropped for it either.
        let pushed = bridges.push(bridge).is_ok();
        if !pushed {
            println!(
                "[dtb] WARNING: tree deeper than {MAX_DEPTH}; anything below {name} may have an \
                 untranslated reg"
            );
        }
        let ancestors = &bridges[..bridges.len() - usize::from(pushed)];

        // `/chosen` and `/cpus` carry properties rather than a `reg`, so they are
        // read here rather than by a targeted lookup of their own.
        if &*path == "/chosen" {
            if let Some((base, size)) = initrd_range(&node) {
                push_foreign(&mut foreign, PhysRange::new("initrd", base, size));
            }
            // Kept, not resolved: the node it names may not have been walked yet.
            console = node.find_property_str("stdout-path").map(console_path);
            continue;
        }
        if &*path == "/cpus" {
            timebase_hz = timebase_hz.or_else(|| timebase_of(&node));
            continue;
        }

        let kind = classify(&node, &path);
        if matches!(kind, RegKind::Ignored) {
            continue;
        }

        let Some(regs) = node.reg() else { continue };

        // A `reg` the parser cannot decode yields no entries at all, which is otherwise
        // indistinguishable from a node without one — and a carve-out dropped in silence is
        // memory the allocator hands out. One- and two-cell addresses and sizes are what it
        // reads; the three cells a PCI bus declares for its children are what land here.
        if regs.clone().next().is_none() {
            println!(
                "[dtb] WARNING: cannot decode {name}'s reg; check its parent's #address-cells \
                 and #size-cells"
            );
            continue;
        }

        // A hart id is not an address: no bus translates it, and it has no size, so it is
        // taken before everything a real range must pass.
        if matches!(kind, RegKind::HartId) {
            timebase_hz = timebase_hz.or_else(|| timebase_of(&node));
            for reg in regs {
                if hart_ids.push(reg.address as usize).is_err() && !harts_dropped {
                    harts_dropped = true;
                    println!(
                        "[dtb] WARNING: machine reports more than the {MAX_HART_IDS} harts this \
                         kernel has cpu slots for; the rest are ignored"
                    );
                }
            }
            continue;
        }

        if matches!(kind, RegKind::Ram) {
            // Only the bank backing the kernel is ours to describe; others are reported
            // rather than dropped, since "one bank" and "the only bank" differ by however
            // much RAM the allocator never hears about.
            for reg in regs {
                let Some(size) = usable_size(name, &reg) else { continue };
                let Some(address) = to_cpu_address(ancestors, reg.address) else {
                    println!(
                        "[dtb] WARNING: {name}'s bank at {:#x} is not an address any \
                         parent bus maps; skipped",
                        reg.address
                    );
                    continue;
                };
                let bank = PhysRange::new(name, phys(address), size);
                if bank.contains(kernel_pa) {
                    ram = Some(bank);
                } else {
                    println!(
                        "[dtb] note: /memory bank {:#x}..{:#x} does not contain the kernel and \
                         is not managed",
                        bank.base,
                        bank.end()
                    );
                }
            }
            continue;
        }

        // A node may describe several ranges — QEMU virt's `flash` has two.
        let mut window = None;
        for reg in regs {
            let Some(size) = usable_size(name, &reg) else { continue };
            let Some(address) = to_cpu_address(ancestors, reg.address) else {
                println!(
                    "[dtb] WARNING: {name}'s reg at {:#x} is not an address any parent bus \
                     maps; skipped",
                    reg.address
                );
                continue;
            };
            let entry = PhysRange::new(name, phys(address), size);

            // Each list has one way in, and it carries the diagnostic for its own bound.
            match kind {
                RegKind::Mmio => {
                    let recorded =
                        Device { base: entry.base.bits(), size: entry.size, irq: irq_of(&node) };
                    match mmio.push(entry) {
                        // The first window this node really contributes. Kept only once the
                        // list has it, so a device resolved below cannot name an address
                        // `kernel_table` never maps.
                        Ok(()) => window = window.or(Some(recorded)),
                        Err(_) => println!(
                            "[dtb] WARNING: more than {MAX_MMIO} MMIO windows; {name} and any \
                             after it are unaccounted for"
                        ),
                    }
                }
                RegKind::ReservedRam => push_foreign(&mut foreign, entry),
                RegKind::HartId | RegKind::Ram | RegKind::Ignored => {
                    unreachable!("handled above")
                }
            }
        }

        // Well-known devices resolve from the node we are already standing on, so `reg` and
        // `interrupts` cannot come from different nodes.
        let Some(device) = window else { continue };
        let compatible = |list: &[&str]| node.compatibles().any(|c| list.contains(&c));
        if compatible(UART) {
            let candidate = UartNode { path: truncated(&path), device };
            if uarts.push(candidate).is_err() {
                println!(
                    "[dtb] note: more than {MAX_UARTS} UARTs; {name} is not a console \
                     candidate"
                );
            }
        } else if plic.is_none() && compatible(PLIC) {
            plic = Some(device);
        } else if clint.is_none() && compatible(CLINT) {
            clint = Some(device);
        }
    }

    let ram = ram.unwrap_or_else(|| {
        panic!("[dtb] /memory has no region containing the kernel at {kernel_pa:#x}")
    });
    let uart = resolve_console(fdt, &uarts, console.as_deref())
        .expect("[dtb] no ns16550-compatible UART node — cannot bring up the console");

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

/// The node path out of a `stdout-path`, whose value is `path` or `path:options`.
fn console_path(stdout: &str) -> String<PATH_LEN> {
    truncated(stdout.split(':').next().unwrap_or(stdout))
}

/// Which of the UARTs found is the console.
///
/// `/chosen/stdout-path` is the tree's own answer, so it wins: a board that lists an
/// unpopulated port ahead of the real one is otherwise a silent failure, since `console`
/// drops the SBI fallback as soon as a base exists. Falling back to the first UART found
/// keeps a tree without `/chosen` bootable.
fn resolve_console(fdt: &Fdt<'_>, uarts: &[UartNode], chosen: Option<&str>) -> Option<Device> {
    let first = uarts.first().map(|uart| uart.device);
    let Some(chosen) = chosen else { return first };

    // `stdout-path` may name an alias instead of a path. Resolving it is a property
    // lookup on a well-known node, not a second search for a device, so it does not
    // reopen the question this module answers in one pass.
    let resolved = if chosen.starts_with('/') {
        truncated(chosen)
    } else {
        match fdt.find_by_path("/aliases").and_then(|node| node.find_property_str(chosen)) {
            Some(path) => console_path(path),
            None => {
                println!(
                    "[dtb] WARNING: /chosen names console '{chosen}', which /aliases \
                          does not define; using the first UART found"
                );
                return first;
            }
        }
    };

    match uarts.iter().find(|uart| uart.path == resolved) {
        Some(uart) => Some(uart.device),
        None => {
            println!(
                "[dtb] WARNING: /chosen names console '{resolved}', which is not a UART this \
                 kernel can drive; using the first UART found"
            );
            first
        }
    }
}

/// The initrd's base and length, if the previous stage loaded one.
///
/// `fdt_raw`'s `Chosen` does not expose it, so the two properties are read directly.
/// Firmware writes them as one cell or two, so both widths are accepted.
fn initrd_range(chosen: &Node<'_>) -> Option<(PhysicalAddr, usize)> {
    let cell = |key: &str| {
        chosen
            .find_property(key)
            .and_then(|prop| prop.as_u64().or_else(|| prop.as_u32().map(u64::from)))
    };
    let start = cell("linux,initrd-start")?;
    let end = cell("linux,initrd-end")?;
    (end > start).then(|| (phys(start), (end - start) as usize))
}
