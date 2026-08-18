//! What a single node says about itself: what its `reg` describes, and the properties the
//! kernel reads off it.
//!
//! Every function here answers from one node and nothing else. Anything needing the node's
//! place in the tree belongs in [`super::bus`].

use fdt_raw::{Node, RegInfo, RegIter};
use mmu::PhysicalAddr;

/// What a node's `reg` property describes.
///
/// Reserved RAM read as a device maps memory the firmware forbids; a device read as RAM hands
/// register space to the allocator.
pub enum RegKind {
    /// A memory-mapped device window.
    Mmio,
    /// RAM carved out by the previous boot stage — present in memory, but not the kernel's to
    /// hand out.
    ReservedRam,
    /// A hart: the `reg` is a **hart id**, not an address at all.
    HartId,
    /// The RAM itself.
    Ram,
}

/// Classify a node by what its `reg` describes — the single place this distinction is made.
///
/// `None` means the `reg` describes nothing the kernel acts on, so the node is left alone
/// rather than guessed at.
///
/// `device_type` is the spec's discriminator for the two kinds of node whose `reg` is not a
/// device window, and QEMU, U-Boot and OpenSBI all emit it. Anything else carrying a `reg` is
/// a device.
///
/// `/reserved-memory/…` are firmware carve-outs, which OpenSBI adds for itself and its PMP
/// then denies to S-mode, so they must reach the frame allocator. They appear only in the
/// tree OpenSBI hands on, so `-machine dumpdtb` does not show them.
pub fn classify(node: &Node<'_>, path: &str) -> Option<RegKind> {
    if path.starts_with("/reserved-memory/") {
        return Some(RegKind::ReservedRam);
    }
    match node.find_property_str("device_type") {
        Some("memory") => Some(RegKind::Ram),
        Some("cpu") => Some(RegKind::HartId),
        // A `reg` under `/cpus` that is not a hart's is neither RAM nor a window the kernel
        // maps.
        _ if path.starts_with("/cpus/") => None,
        _ => Some(RegKind::Mmio),
    }
}

/// Whether the OS should ignore this node and everything under it.
///
/// Absent `status` means enabled, per the spec; `disabled`, `fail`, `fail-sss` and `reserved`
/// all mean "not yours", so anything not explicitly fine is skipped.
pub fn is_disabled(node: &Node<'_>) -> bool {
    !matches!(node.find_property_str("status"), None | Some("okay") | Some("ok"))
}

/// The node's `reg` entries, or `None` when there are none to read.
///
/// A `reg` the parser cannot decode yields no entries at all, which is otherwise
/// indistinguishable from a node without one — and a carve-out dropped in silence is memory
/// the allocator hands out. It reads one- and two-cell addresses and sizes; the three cells a
/// PCI bus declares for its children are what land here.
pub fn decoded_regs<'a>(node: &Node<'a>, name: &str) -> Option<RegIter<'a>> {
    let regs = node.reg()?;
    if regs.clone().next().is_none() {
        println!(
            "[dtb] WARNING: cannot decode {name}'s reg; check its parent's #address-cells \
             and #size-cells"
        );
        return None;
    }
    Some(regs)
}

/// A `reg` entry's length, or `None` with a diagnostic.
///
/// Loud in every case, and shared by every kind of range: a device with no window is a driver
/// that cannot bind, and a carve-out dropped in silence is memory the allocator hands out. A
/// missing size usually means `#size-cells = <0>`.
pub fn usable_size(name: &str, reg: &RegInfo) -> Option<usize> {
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
/// `#interrupt-cells`, and that node is reached by phandle rather than from here; under a two-
/// or three-cell encoding the first cell is a type or a flags word, not a number the kernel
/// could program. `interrupts-extended` is likewise absent, since it names a controller per
/// entry. Reporting nothing beats reporting a number that means something else.
pub fn irq_of(node: &Node<'_>) -> Option<usize> {
    let property = node.find_property("interrupts")?;
    let mut cells = property.as_u32_iter();
    let first = cells.next()?;
    cells.next().is_none().then_some(first as usize)
}

/// `timebase-frequency`, which the spec allows on `/cpus` or on any hart below it.
pub fn timebase_of(node: &Node<'_>) -> Option<usize> {
    node.find_property("timebase-frequency")
        .and_then(|property| property.as_u32().map(u64::from).or_else(|| property.as_u64()))
        .map(|hz| hz as usize)
}

/// The initrd's base and length, if the previous stage loaded one.
///
/// `fdt_raw`'s `Chosen` does not expose it, so the two properties are read directly. Firmware
/// writes them as one cell or two, so both widths are accepted.
pub fn initrd_range(chosen: &Node<'_>) -> Option<(PhysicalAddr, usize)> {
    let cell = |key: &str| {
        chosen
            .find_property(key)
            .and_then(|prop| prop.as_u64().or_else(|| prop.as_u32().map(u64::from)))
    };
    let start = cell("linux,initrd-start")?;
    let end = cell("linux,initrd-end")?;
    (end > start).then(|| (phys(start), (end - start) as usize))
}

/// A `reg` cell as a physical address — the one place the tree's raw integers become one.
pub fn phys(address: u64) -> PhysicalAddr { PhysicalAddr::new(address as usize) }
