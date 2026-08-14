//! Flattened Device Tree (FDT / DTB) discovery.
//!
//! Firmware hands the boot hart the blob's physical address in `a1`; we parse it here
//! with the zero-allocation [`fdt_raw`] crate. No heap needed, so this runs before
//! `memory::init` — in fact before anything prints, since the console learns the UART
//! base from here.
//!
//! One walk, one table: [`walk`] traverses, [`table`] keeps the single copy, [`report`]
//! prints. Everything the kernel needs comes from the accessors below, never from a
//! platform constant; memory bring-up takes all of its share in one go, as
//! [`machine_memory`].
//!
//! **Known gap:** a `reg` address is used as a CPU physical address directly, which is
//! silently wrong where `/soc` declares a non-identity `ranges`. `Fdt::translate_address`
//! borrows the blob while `Node::path()` returns an owned string, so composing them means
//! reimplementing the `ranges` walk here. QEMU virt is identity, so it has never bitten.

mod report;
mod table;
mod walk;

pub use report::summary;

use fdt_raw::Fdt;
use paging::PhysicalAddr;

use crate::memory::MachineMemory;

/// Parse the device tree at `dtb_ptr` and record the hardware the kernel needs.
///
/// Must run before the first print, since the console's UART base comes from here; does
/// not print itself — that is [`summary`]. A usable tree is part of the boot contract, so
/// a null pointer, an unparseable blob, a `/memory` without the kernel in it or a missing
/// UART all panic rather than limp on wrong addresses.
///
/// # Safety
/// `dtb_ptr` must be a valid, readable FDT blob (as passed in `a1`) that stays mapped and
/// unmodified — it is borrowed in place.
pub unsafe fn init(dtb_ptr: usize) {
    if dtb_ptr == 0 {
        panic!(
            "[dtb] no device tree pointer in a1 — previous boot stage violated the boot contract"
        );
    }

    // `a1` is *physical*, so reach the blob through the direct map: both tables map it
    // there, while the raw address needs an identity mapping that no longer exists.
    let blob = crate::memory::phys_to_virt(PhysicalAddr::new(dtb_ptr)).as_mut_ptr::<u8>();
    // SAFETY: forwarded from this function's contract — `dtb_ptr` addresses a valid
    // FDT that stays mapped and unmodified, and `blob` is its direct-map alias.
    let fdt = unsafe { Fdt::from_ptr(blob) }
        .unwrap_or_else(|error| panic!("[dtb] failed to parse FDT at {dtb_ptr:#x}: {error:?}"));

    // Which `/memory` bank is ours: the one holding our own load address.
    let kernel_pa = crate::memory::virt_to_phys(crate::memory::layout::text_start());
    let size = fdt.header().totalsize as usize;

    table::TABLE.call_once(|| walk::discover(&fdt, PhysicalAddr::new(dtb_ptr), size, kernel_pa));
}

/// Everything [`crate::memory::init`] needs to know about physical memory, as one value.
///
/// The whole of this module's contribution to memory bring-up, in one call: `memory` reads
/// nothing out of the tree itself, so a board without an FDT means another builder of this
/// struct and no change over there.
///
/// # Panics
///
/// Before [`init`], since none of it is known until the tree has been walked.
pub fn machine_memory() -> MachineMemory<'static> {
    let table = table::get().expect("device tree not parsed; call device_tree::init first");
    MachineMemory {
        ram_end: table.ram.end,
        foreign: table.foreign.as_slice(),
        mmio: table.mmio.as_slice(),
    }
}

/// Primary UART base. No address is hardcoded anywhere — the console falls back to the
/// SBI console until this is known.
pub fn uart_base() -> Option<usize> { table::get().map(|t| t.uart.base) }

/// Ticks per second of the `time` CSR, or `None` if the tree did not say.
///
/// `Option` although the binding requires it: a caller needing a bounded wait decides
/// what to do without a clock rather than being handed a fabricated frequency.
pub fn timebase_hz() -> Option<usize> { table::get().and_then(|t| t.timebase_hz) }

/// Every hart id the machine reports, from `/cpus/cpu@N`'s `reg`.
///
/// A list to iterate, never a count and never an array index — ids need not be `0..n`;
/// see [`crate::memory::stack`]. Harts not `status = "okay"` are absent, since a
/// disabled core cannot be started.
pub fn hart_ids() -> &'static [usize] { table::get().map(|t| t.hart_ids.as_slice()).unwrap_or(&[]) }
