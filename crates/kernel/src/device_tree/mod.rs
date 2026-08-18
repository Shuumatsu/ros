//! Flattened Device Tree (FDT / DTB) discovery.
//!
//! Firmware hands the boot hart the blob's physical address in `a1`; the zero-allocation
//! [`fdt_raw`] crate parses it in place, before the heap and before anything else prints —
//! the console starts on the SBI console and binds the real UART the moment this yields its
//! base.
//!
//! One walk, one table: [`walk`] traverses, [`table`] keeps the single copy, [`report`]
//! prints. Everything the kernel needs comes from the accessors below, never from a platform
//! constant; memory bring-up takes its whole share as [`machine_memory`].
//!
//! A bridge's own `ranges` is an aperture its children may sit in, not a window any device
//! occupies, so it contributes no MMIO window — the PCI host on QEMU virt publishes 17 GiB
//! of address space for devices the tree never describes. Whoever brings up PCI takes that
//! aperture from the bridge node and maps the parts it assigns. `ranges` is still followed
//! for translation, so a child's `reg` is recorded as the address the CPU issues.
//!
//! Timer and IPI go through SBI, and a PLIC is programmed by the driver that claims it, so
//! no interrupt controller is resolved by name. Names belong to drivers: [`crate::drivers`]
//! holds every `compatible` string [`walk`] compares against.

mod bus;
mod console;
mod props;
mod report;
mod table;
mod walk;

pub use report::summary;

use fdt_raw::{Fdt, Header};
use mmu::PhysicalAddr;

use crate::memory::machine::MachineMemory;
use crate::memory::{direct_map, layout};

/// Parse the device tree at `base` and record the hardware the kernel needs.
///
/// A usable tree is part of the boot contract: a null pointer, a blob the direct map cannot
/// reach, an unparseable blob, a `/memory` without the kernel in it and a missing UART all
/// panic rather than let the kernel limp on wrong addresses.
///
/// # Safety
/// `base` must begin a valid, readable FDT blob (as passed in `a1`) that stays mapped and
/// unmodified for the duration of this call — it is borrowed in place. Nothing outlives the
/// call: [`table`] keeps copies.
pub unsafe fn init(base: PhysicalAddr) {
    assert!(
        table::get().is_none(),
        "device_tree::init called twice; the table is already published"
    );
    if base.bits() == 0 {
        panic!(
            "[dtb] no device tree pointer in a1 — previous boot stage violated the boot contract"
        );
    }

    // The blob carries its own length, so the header is what says how far the rest reaches,
    // and nothing may borrow the rest until that reach has been checked: `Fdt::from_ptr`
    // builds a slice over the whole blob, so it comes after the second check.
    direct_map::require_reach("the device tree header", base, size_of::<Header>());
    let blob = direct_map::phys_to_virt(base).as_mut_ptr::<u8>();
    // SAFETY: forwarded from this function's contract — `dtb_ptr` addresses a valid FDT
    // that stays mapped and unmodified, and `blob` is its direct-map alias. Only the
    // header is read here, which the check above covers.
    let header = unsafe { Header::from_ptr(blob) }.unwrap_or_else(|error| {
        panic!("[dtb] failed to parse the FDT header at {base:#x}: {error:?}")
    });
    let size = header.totalsize as usize;
    direct_map::require_reach("the device tree blob", base, size);
    // SAFETY: as above, and the whole blob is now known to lie inside the direct map.
    let fdt = unsafe { Fdt::from_ptr(blob) }
        .unwrap_or_else(|error| panic!("[dtb] failed to parse FDT at {base:#x}: {error:?}"));

    // Which `/memory` bank is ours: the one holding our own load address.
    let kernel_pa = direct_map::virt_to_phys(layout::text_start());

    table::TABLE.call_once(|| walk::discover(&fdt, base, size, kernel_pa));

    // Checked here rather than with the rest of the windows: the console binds this one on
    // its very next line, well before `MachineMemory::check` reaches it.
    let uart = table::get().expect("the table was published above").uart;
    direct_map::require_reach("the console UART window", uart.base, uart.size);
}

/// Everything [`crate::memory`] needs to know about physical memory, as one value.
///
/// `memory` reads nothing out of the tree itself, so a board without an FDT means another
/// builder of this struct and no change over there.
///
/// # Panics
///
/// Before [`init`], since none of it is known until the tree has been walked.
pub fn machine_memory() -> MachineMemory<'static> {
    let table = table::get().expect("device tree not parsed; call device_tree::init first");
    MachineMemory {
        ram: &table.ram,
        foreign: table.foreign.as_slice(),
        mmio: table.mmio.as_slice(),
    }
}

/// Primary UART base. No address is hardcoded anywhere — the console falls back to the
/// SBI console until this is known.
pub fn uart_base() -> Option<PhysicalAddr> { table::get().map(|t| t.uart.base) }

/// Ticks per second of the `time` CSR, or `None` if the tree did not say. A caller needing a
/// bounded wait decides what to do without a clock rather than being handed a fabricated
/// frequency.
pub fn timebase_hz() -> Option<usize> { table::get().and_then(|t| t.timebase_hz) }

/// Every hart id the machine reports, from `/cpus/cpu@N`'s `reg`.
///
/// A list to iterate, never a count and never an array index — ids need not be `0..n`;
/// see [`crate::memory::stack`]. Harts not `status = "okay"` are absent.
pub fn hart_ids() -> &'static [usize] { table::get().map(|t| t.hart_ids.as_slice()).unwrap_or(&[]) }
