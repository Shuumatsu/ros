//! Flattened Device Tree (FDT / DTB) discovery.
//!
//! Firmware hands the boot hart the blob's physical address in `a1`; we parse it here with
//! the zero-allocation [`fdt_raw`] crate, which is what lets this run before the heap and
//! before `memory::init_allocators`. It also runs before anything else prints: the console
//! starts on the SBI console and binds the real UART the moment this yields its base, so
//! walking the tree first is what puts every line on the real port.
//!
//! One walk, one table: [`walk`] traverses, [`table`] keeps the single copy, [`report`]
//! prints. Everything the kernel needs comes from the accessors below, never from a
//! platform constant; memory bring-up takes all of its share in one go, as
//! [`machine_memory`].
//!
//! **What is deliberately not collected:** a bridge's own `ranges` describe an aperture its
//! children may sit in, not a window any device occupies — the PCI host on QEMU virt
//! publishes 1 GiB at `0x4000_0000` and 16 GiB above `0x4_0000_0000`, address space for
//! devices the tree does not describe at all. [`machine_memory`] is the list
//! [`crate::memory::kernel_table`] maps in full, and what belongs in it is the register
//! windows a `reg` names. Whoever brings up PCI takes the aperture from the bridge node and
//! maps the parts it assigns. `ranges` *is* followed for translation, so a child's `reg` is
//! recorded as the address the CPU issues.
//!
//! No interrupt controller is resolved by name either. Timer and IPI go through SBI, and
//! under this boot model the CLINT's registers are M-mode's, so a driver holding that base
//! would be holding one it must not use. A PLIC is S-mode's to program, and resolving it
//! comes with the driver that claims it. Both windows stay in the MMIO list like any other,
//! since that list says what the machine has rather than what S-mode may touch.
//!
//! A node is matched by name only on behalf of a driver, and the names are the driver's own:
//! [`crate::drivers`] holds every `compatible` string [`walk`] compares against, so a chip
//! this kernel learns to drive is one module's worth of change.

mod report;
mod table;
mod walk;

pub use report::summary;

use fdt_raw::{Fdt, Header};
use mmu::PhysicalAddr;

use crate::memory::machine::MachineMemory;
use crate::memory::{direct_map, layout};

/// Parse the device tree at `dtb_ptr` and record the hardware the kernel needs.
///
/// Runs before the console has a UART of its own, since that base comes from here; what it
/// reports on the way goes out through SBI. A usable tree is part of the boot contract, so
/// a null pointer, a blob the direct map cannot reach, an unparseable blob, a `/memory`
/// without the kernel in it or a missing UART all panic rather than limp on wrong addresses.
///
/// # Safety
/// `dtb_ptr` must be a valid, readable FDT blob (as passed in `a1`) that stays mapped and
/// unmodified for the duration of this call — it is borrowed in place. Nothing outlives the
/// call: [`table`] keeps copies.
pub unsafe fn init(dtb_ptr: usize) {
    assert!(
        table::get().is_none(),
        "device_tree::init called twice; the table is already published"
    );
    if dtb_ptr == 0 {
        panic!(
            "[dtb] no device tree pointer in a1 — previous boot stage violated the boot contract"
        );
    }

    // `a1` is physical, and the direct map is how this kernel turns a physical address into a
    // pointer: `phys_to_virt` is a compile-time add and holds under the boot table and the
    // kernel table alike. Its window is bounded, so the blob has to be inside it — beyond the
    // end nothing is mapped, and the read below would park this hart instead of reporting.
    let base = PhysicalAddr::new(dtb_ptr);
    // Header first, and on its own: the blob carries its own length, so the header is what
    // says how far the rest reaches, and nothing may borrow the rest until that reach has
    // been checked. `Fdt::from_ptr` builds a slice over the whole blob, so it comes after
    // the second check rather than before it.
    direct_map::require_reach("the device tree header", base, size_of::<Header>());
    let blob = direct_map::phys_to_virt(base).as_mut_ptr::<u8>();
    // SAFETY: forwarded from this function's contract — `dtb_ptr` addresses a valid FDT
    // that stays mapped and unmodified, and `blob` is its direct-map alias. Only the
    // header is read here, which the check above covers.
    let header = unsafe { Header::from_ptr(blob) }.unwrap_or_else(|error| {
        panic!("[dtb] failed to parse the FDT header at {dtb_ptr:#x}: {error:?}")
    });
    let size = header.totalsize as usize;
    direct_map::require_reach("the device tree blob", base, size);
    // SAFETY: as above, and the whole blob is now known to lie inside the direct map.
    let fdt = unsafe { Fdt::from_ptr(blob) }
        .unwrap_or_else(|error| panic!("[dtb] failed to parse FDT at {dtb_ptr:#x}: {error:?}"));

    // Which `/memory` bank is ours: the one holding our own load address.
    let kernel_pa = direct_map::virt_to_phys(layout::text_start());

    table::TABLE.call_once(|| walk::discover(&fdt, base, size, kernel_pa));

    // The console binds this window on its very next line, which is well before
    // `MachineMemory::check` checks every window the machine describes. Checked here, where the
    // fact becomes usable, rather than where the rest of them are.
    let uart = table::get().expect("the table was published above").uart;
    direct_map::require_reach("the console UART window", uart.base, uart.size);
}

/// Everything [`crate::memory`] needs to know about physical memory, as one value.
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
        ram: &table.ram,
        foreign: table.foreign.as_slice(),
        mmio: table.mmio.as_slice(),
    }
}

/// Primary UART base. No address is hardcoded anywhere — the console falls back to the
/// SBI console until this is known.
pub fn uart_base() -> Option<PhysicalAddr> { table::get().map(|t| t.uart.base) }

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
