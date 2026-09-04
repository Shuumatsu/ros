//! Typed reads of individual node properties, warning for values this kernel cannot use.

use fdt_raw::{Node, RegInfo, RegIter, Status};
use mmu::PhysicalAddr;

pub enum RegKind {
    Mmio,
    ReservedRam,
    HartId,
    Ram,
}

/// Classify a node as reserved RAM by path, RAM or hart ID by `device_type`, and MMIO
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

/// Treat a node as enabled only when `status` is absent or reports an operational device.
pub fn is_disabled(node: &Node<'_>) -> bool {
    node.find_property("status").is_some_and(|status| status.as_status() != Some(Status::Okay))
}

/// Return decoded `reg` entries, warning when the decoder produces no entries.
pub fn decoded_regs<'a>(node: &Node<'a>) -> Option<RegIter<'a>> {
    let regs = node.reg()?;
    if regs.clone().next().is_none() {
        println!(
            "[dtb] WARNING: cannot decode {}'s reg; check its parent's #address-cells \
             and #size-cells",
            node.name()
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
    Some(node.find_property("interrupts")?.as_u32()? as usize)
}

/// Read a scalar the specification allows in either a 32- or a 64-bit encoding.
fn scalar(node: &Node<'_>, name: &str) -> Option<u64> {
    let property = node.find_property(name)?;
    property.as_u32().map(u64::from).or_else(|| property.as_u64())
}

pub fn timebase_of(node: &Node<'_>) -> Option<u64> { scalar(node, "timebase-frequency") }

/// Parse initrd bounds, returning only nonempty ranges.
pub fn initrd_range(chosen: &Node<'_>) -> Option<(PhysicalAddr, usize)> {
    let start = scalar(chosen, "linux,initrd-start")?;
    let end = scalar(chosen, "linux,initrd-end")?;
    (end > start).then(|| (PhysicalAddr::new(start as usize), (end - start) as usize))
}
