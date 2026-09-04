use fdt_raw::{Node, RegInfo, RegIter};
use mmu::PhysicalAddr;

pub enum RegKind {
    Mmio,
    ReservedRam,
    HartId,
    Ram,
}

/// Classify `reg` as reserved RAM by path, RAM or hart ID by `device_type`, and MMIO
/// otherwise. Other children below `/cpus` are ignored.
pub fn classify(node: &Node<'_>, path: &str) -> Option<RegKind> {
    if path.starts_with("/reserved-memory/") {
        return Some(RegKind::ReservedRam);
    }
    match node.find_property_str("device_type") {
        Some("memory") => Some(RegKind::Ram),
        Some("cpu") => Some(RegKind::HartId),
        _ if path.starts_with("/cpus/") => None,
        _ => Some(RegKind::Mmio),
    }
}

/// Treat a node as enabled only when `status` is absent, `"okay"`, or `"ok"`.
pub fn is_disabled(node: &Node<'_>) -> bool {
    !matches!(node.find_property_str("status"), None | Some("okay") | Some("ok"))
}

/// Return decoded `reg` entries, warning when the decoder produces no entries.
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

/// Return a nonzero `reg` length, warning for missing or zero lengths.
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

/// Accept only a single-cell `interrupts` value; ignore controller-specific multi-cell
/// encodings and `interrupts-extended`.
pub fn irq_of(node: &Node<'_>) -> Option<usize> {
    let property = node.find_property("interrupts")?;
    let mut cells = property.as_u32_iter();
    let first = cells.next()?;
    cells.next().is_none().then_some(first as usize)
}

pub fn timebase_of(node: &Node<'_>) -> Option<usize> {
    node.find_property("timebase-frequency")
        .and_then(|property| property.as_u32().map(u64::from).or_else(|| property.as_u64()))
        .map(|hz| hz as usize)
}

/// Parse 32- or 64-bit initrd bounds, returning only nonempty ranges.
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

pub fn phys(address: u64) -> PhysicalAddr { PhysicalAddr::new(address as usize) }
