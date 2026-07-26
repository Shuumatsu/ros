//! The kernel's direct map — `VA = PA + `[`VA_OFFSET`] for all of physical memory.
//!
//! This module is the single source for the mapping `boot.S` installs. Three
//! facts used to be spread across assembly, the linker script and `frame.rs`,
//! each re-deriving them its own way; they live here now:
//!
//! 1. [`VA_OFFSET`] — the direct-map base. Mirrored by `kernel.ld`'s
//!    `_va_offset`, and checked against reality at boot by [`verify`].
//! 2. [`EARLY_PGTABLE`] / [`EARLY_SATP_TEMPLATE`] — the boot page table and the
//!    `satp` value that installs it, both built by `paging` at **compile time**,
//!    so no PTE format and no translation mode appears in `boot.S`.
//! 3. [`WINDOW_END`] — how much physical memory those mappings actually reach,
//!    which is what bounds the frame allocator (see [`super::frame::init`]).
//!
//! # Why a *linear* map
//!
//! `VA = PA + VA_OFFSET` holds unconditionally, with no RAM base subtracted out,
//! which buys two things:
//!
//! - [`super::phys_to_virt`] is a compile-time add — no runtime offset to record
//!   and no window in which it silently returns garbage because it has not been
//!   recorded yet.
//! - It is valid for *every* physical address, MMIO included. The predecessor
//!   skewed the offset by the RAM base (`0xffffffbf80000000`), so
//!   `phys_to_virt(0x1000_0000)` — the UART — produced `0xffffffbf90000000`,
//!   which is not even a canonical Sv39 address. It never faulted only because
//!   nothing called it on a device address. That trap is now gone, which is what
//!   makes eventually dropping the identity map possible.
//!
//! # Why it is `const`
//!
//! A root-level leaf has no intermediate tables beneath it, so
//! [`Table::map_gigapage`] allocates nothing and is `const`. The whole table is
//! therefore a `static` materialized by the loader: there is no early allocator
//! to bootstrap, and no Rust needs to run before paging is on — which sidesteps
//! the pre-paging codegen hazards (`gp`-relative relaxation, absolute
//! relocations) entirely. The table is *data*, not code.
//!
//! Because every leaf pre-sets `A`/`D`, the hardware walker never writes to the
//! table, so it is genuinely immutable and lives in `.rodata`.

use paging::sv39::{PteFlags, ROOT_LEVEL, page_size_at};
use paging::{PhysicalAddr, Satp, Table, VirtualAddr};

/// Bottom of the Sv39 high half, and the base of the kernel's direct map.
///
/// Duplicated in `kernel.ld` as `_va_offset` out of necessity — the linker
/// cannot read a Rust `const`, and parsing the linker script from a `build.rs`
/// trades this for worse glue. [`verify`] is what keeps the duplicate honest.
pub const VA_OFFSET: usize = 0xffff_ffc0_0000_0000;

/// Bytes mapped by one root-level leaf.
const GIGAPAGE: usize = page_size_at(ROOT_LEVEL);

/// Root-level leaves the boot table installs, counting up from physical 0.
///
/// Four is not tuning: it is the smallest window that covers low MMIO *and* the
/// RAM base on every RISC-V platform we target, while still fitting in a table
/// we can afford to build at compile time.
const WINDOW_GIGAPAGES: usize = 4;

/// One past the highest physical address the boot mappings reach.
///
/// Frames above this are addressable through neither boot mapping, so the frame
/// allocator must not hand them out. This is the constant `frame.rs` used to
/// re-derive as `device_tree::ram_base() + 1 GiB`.
pub const WINDOW_END: usize = WINDOW_GIGAPAGES * GIGAPAGE;

/// Permissions for a boot mapping: full access, with `A`/`D` pre-set so the
/// hardware walker never needs to write back into the table.
const BOOT: PteFlags =
    PteFlags::READ_WRITE_EXECUTE.union(PteFlags::ACCESS).union(PteFlags::DIRTY);

/// Build the early page table at compile time.
///
/// Each of the low [`WINDOW_GIGAPAGES`] gigapages of physical memory is mapped
/// **twice**:
///
/// - *identity*, so the instruction stream survives the `csrw satp` that turns
///   translation on while the PC is still a physical address, and so MMIO stays
///   reachable at its raw address (the console does exactly that);
/// - at `VA_OFFSET + PA`, the direct map — the kernel's durable home, which is
///   where it runs from the moment `boot.S` jumps high.
///
/// Note what is *absent*: the RAM base. The window starts at physical 0 and RAM
/// lands wherever the platform puts it inside that window, so nothing here — and
/// therefore nothing in `boot.S` — needs to know where DRAM begins.
const fn early_table() -> Table {
    let mut table = Table::new();
    let mut i = 0;
    while i < WINDOW_GIGAPAGES {
        let pa = PhysicalAddr::new(i * GIGAPAGE);
        table.map_gigapage(VirtualAddr::new(i * GIGAPAGE), pa, BOOT);
        table.map_gigapage(VirtualAddr::new(VA_OFFSET + i * GIGAPAGE), pa, BOOT);
        i += 1;
    }
    table
}

/// The root page table `boot.S` installs. Named by `boot.S`, hence `no_mangle`.
#[used]
#[unsafe(no_mangle)]
static EARLY_PGTABLE: Table = early_table();

/// The `satp` value `boot.S` writes, with `PPN` left zero for it to fill in.
///
/// The root table's physical address is a link-time fact only a PC-relative
/// `lla` can recover, so it cannot appear in a `const`. Everything else — the
/// Sv39 mode encoding above all — comes from `paging`, and `boot.S` just `or`s
/// the page number in. See [`Satp::with_root`], which is the same operation.
#[used]
#[unsafe(no_mangle)]
static EARLY_SATP_TEMPLATE: usize = Satp::sv39(PhysicalAddr::new(0), 0).bits();

/// Assert the direct map Rust believes in is the one we are actually running on.
///
/// `boot.S` measures `VA - PA` as the *linked* address of its high-half
/// continuation minus the physical address it actually reached that code at —
/// a fact about reality, not about intent, so this catches a mismatched linker
/// script and a loader that put us somewhere unexpected alike.
///
/// Call once, before anything translates.
pub fn verify(measured: usize) {
    assert_eq!(
        measured, VA_OFFSET,
        "boot.S measured a VA offset of {measured:#x}, but the direct map is built for \
         {VA_OFFSET:#x}; kernel.ld's _va_offset and memory::direct_map::VA_OFFSET have diverged"
    );
}
