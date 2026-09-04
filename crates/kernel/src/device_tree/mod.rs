//! Flattened Device Tree discovery and resolved hardware metadata.

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

/// Parse the device tree and publish the discovered hardware table.
///
/// Invalid or unreachable blobs, missing kernel RAM, and missing supported UARTs are fatal.
///
/// # Safety
/// `base` must address a valid, readable FDT blob that remains mapped and unmodified for this
/// call. The published table contains no borrows from the blob.
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

    // `Fdt::from_ptr` forms a slice over `totalsize`, so validate the header and full extent
    // before constructing it.
    direct_map::require_reach("the device tree header", base, size_of::<Header>());
    let blob = direct_map::phys_to_virt(base).as_mut_ptr::<u8>();
    // SAFETY: `blob` is the direct-map alias guaranteed by the caller, and the header extent
    // was validated above.
    let header = unsafe { Header::from_ptr(blob) }.unwrap_or_else(|error| {
        panic!("[dtb] failed to parse the FDT header at {base:#x}: {error:?}")
    });
    let size = header.totalsize as usize;
    direct_map::require_reach("the device tree blob", base, size);
    // SAFETY: the caller's guarantees apply, and the full blob extent was validated above.
    let fdt = unsafe { Fdt::from_ptr(blob) }
        .unwrap_or_else(|error| panic!("[dtb] failed to parse FDT at {base:#x}: {error:?}"));

    let kernel_pa = direct_map::virt_to_phys(layout::text_start());

    table::TABLE.call_once(|| walk::discover(&fdt, base, size, kernel_pa));

    let uart = table::get().expect("the table was published above").uart;
    direct_map::require_reach("the console UART window", uart.base, uart.size);
}

/// Return the discovered RAM, reserved ranges, and MMIO windows.
///
/// # Panics
/// Panics before [`init`].
pub fn machine_memory() -> MachineMemory<'static> {
    let table = table::get().expect("device tree not parsed; call device_tree::init first");
    MachineMemory {
        ram: &table.ram,
        foreign: table.foreign.as_slice(),
        mmio: table.mmio.as_slice(),
    }
}

pub fn uart_base() -> Option<PhysicalAddr> { table::get().map(|t| t.uart.base) }

pub fn timebase_hz() -> Option<usize> { table::get().and_then(|t| t.timebase_hz) }

/// Enabled firmware-reported hart IDs, which need not be contiguous or zero-based.
pub fn hart_ids() -> &'static [usize] { table::get().map(|t| t.hart_ids.as_slice()).unwrap_or(&[]) }
