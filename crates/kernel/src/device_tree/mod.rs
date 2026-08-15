//! Flattened Device Tree (FDT / DTB) discovery.
//!
//! Firmware hands the boot hart the blob's physical address in `a1`; we parse it here with
//! the zero-allocation [`fdt_raw`] crate, which is what lets this run before the heap and
//! before `memory::init`. It also runs before anything else prints: the console starts on
//! the SBI console and binds the real UART the moment this yields its base, so walking the
//! tree first is what puts every line on the real port.
//!
//! One walk, one table: [`walk`] traverses, [`table`] keeps the single copy, [`report`]
//! prints. Everything the kernel needs comes from the accessors below, never from a
//! platform constant; memory bring-up takes all of its share in one go, as
//! [`machine_memory`].
//!
//! **Known gap:** a `reg` address is used as a CPU physical address directly, which is
//! silently wrong under a parent bus whose `ranges` is not identity.
//! `Fdt::translate_address` wants a path borrowed for the blob's lifetime while
//! `Node::path()` returns an owned string, so composing them means reimplementing the
//! `ranges` walk here. On QEMU virt `/soc` translates one-to-one, and `platform-bus` — the
//! one node that does not — has no children.

mod report;
mod table;
mod walk;

pub use report::summary;

use fdt_raw::{Fdt, Header};
use paging::PhysicalAddr;

use crate::memory::MachineMemory;
use crate::memory::direct_map::DIRECT_MAP_END;
use crate::utils::ByteSize;

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
    // Two steps, because the blob carries its own length: the header has to be readable
    // before the rest can be measured.
    require_reachable(base, size_of::<Header>());
    let blob = crate::memory::phys_to_virt(base).as_mut_ptr::<u8>();
    // SAFETY: forwarded from this function's contract — `dtb_ptr` addresses a valid
    // FDT that stays mapped and unmodified, and `blob` is its direct-map alias.
    let fdt = unsafe { Fdt::from_ptr(blob) }
        .unwrap_or_else(|error| panic!("[dtb] failed to parse FDT at {dtb_ptr:#x}: {error:?}"));
    let size = fdt.header().totalsize as usize;
    require_reachable(base, size);

    // Which `/memory` bank is ours: the one holding our own load address.
    let kernel_pa = crate::memory::virt_to_phys(crate::memory::layout::text_start());

    table::TABLE.call_once(|| walk::discover(&fdt, base, size, kernel_pa));
}

/// Require the blob to lie inside the direct map's window, which is all the physical memory
/// the kernel can name.
fn require_reachable(base: PhysicalAddr, size: usize) {
    let end = base.bits().saturating_add(size);
    assert!(
        end <= DIRECT_MAP_END.bits(),
        "[dtb] the blob at {base:#x}..{end:#x} lies past the direct map's {} window; raise \
         memory::direct_map::DIRECT_MAP_SPAN",
        ByteSize(DIRECT_MAP_END.bits())
    );
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
