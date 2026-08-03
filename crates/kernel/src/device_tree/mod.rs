//! Flattened Device Tree (FDT / DTB) discovery.
//!
//! The previous boot stage (QEMU with `-bios none`, or an SBI firmware) hands us
//! the physical address of the device tree blob in register `a1`. `boot.S`
//! preserves it, `start(dtb)` threads it through, and we parse it here with the
//! zero-allocation [`fdt_raw`] crate. No heap is required, so discovery runs
//! before `memory::init` brings the allocator up — in fact before *anything*
//! prints, because the console itself learns the UART address from here.
//!
//! # Shape
//!
//! One walk, one table, four files:
//!
//! | file | job |
//! |---|---|
//! | [`region`] | `PhysRegion`, a named physical range |
//! | [`table`] | what a walk produces, and the single copy of it |
//! | [`walk`] | the one traversal that fills it |
//! | [`report`] | printing it, after the console exists |
//!
//! With parsing in one file and storage in another there is nowhere for a second
//! parser to quietly appear, which is the failure this shape exists to prevent.
//! [`table`] and [`walk`] each carry the specifics.
//!
//! Everything the rest of the kernel needs — RAM extent, UART base, MMIO windows,
//! foreign RAM, hart ids, timebase — comes from the accessors below, not from
//! hardcoded platform constants.
//!
//! # Known gap: no `ranges` translation
//!
//! A `reg` address is used as a CPU physical address directly. On a platform whose
//! `/soc` declares a non-identity `ranges`, that is wrong — and wrong *silently*,
//! since the untranslated address still gets mapped and the kernel just drives the
//! wrong part of the bus. `fdt_raw::Fdt::translate_address` exists for this but
//! takes `&'a str` tied to the blob's borrow, while `Node::path()` returns an owned
//! `heapless::String<256>`, so the two do not compose without reimplementing the
//! `ranges` walk here. QEMU virt's `/soc` has an empty (identity) `ranges`, which is
//! why this has never bitten. Left undone deliberately rather than half-done.

mod region;
mod report;
mod table;
mod walk;

pub use region::PhysRegion;
pub use report::summary;

use fdt_raw::Fdt;

/// Parse the device tree at `dtb_ptr` and record the hardware the kernel needs.
/// Populating the UART base here is what backs the console, so this must run
/// before the first print — but `init` does not print itself; call [`summary`]
/// for that.
///
/// A usable device tree is part of the boot contract: a null pointer, an
/// unparseable blob, a `/memory` with no region containing the kernel, or a
/// missing UART all **panic** rather than let the kernel limp on wrong
/// addresses. (Such a panic is still visible via the SBI console.)
///
/// # Safety
/// `dtb_ptr` must be the address of a valid, readable FDT blob (as passed in
/// `a1`), and the blob must stay mapped and unmodified — we borrow it in place.
pub unsafe fn init(dtb_ptr: usize) {
    if dtb_ptr == 0 {
        panic!("[dtb] no device tree pointer in a1 — previous boot stage violated the boot contract");
    }

    // `a1` holds a *physical* address; reach the blob through the direct map rather
    // than treating it as a pointer. Both the boot table and the kernel table map it
    // there, whereas the raw address is only dereferenceable while a boot identity
    // mapping exists — and it no longer does.
    let blob = crate::memory::phys_to_virt(dtb_ptr) as *mut u8;
    // SAFETY: forwarded from this function's contract — `dtb_ptr` addresses a valid
    // FDT that stays mapped and unmodified, and `blob` is its direct-map alias.
    let fdt = unsafe { Fdt::from_ptr(blob) }
        .unwrap_or_else(|error| panic!("[dtb] failed to parse FDT at {dtb_ptr:#x}: {error:?}"));

    // Which `/memory` bank is ours: the one holding our own load address.
    let kernel_pa = crate::memory::virt_to_phys(crate::memory::layout::text_start());
    let size = fdt.header().totalsize as usize;

    table::TABLE.call_once(|| walk::discover(&fdt, dtb_ptr, size, kernel_pa));
}

/// Exclusive end of the RAM region backing the kernel — the authoritative RAM
/// top. Prefer it over the linker's compile-time estimate.
pub fn ram_end() -> Option<usize> {
    table::get().map(|t| t.ram.end)
}

/// Primary UART base, or `None` before the device tree has been parsed. There is
/// no hardcoded UART address: the console falls back to the SBI console until
/// this is known, then uses the DTB-reported MMIO base.
pub fn uart_base() -> Option<usize> {
    table::get().map(|t| t.uart.base)
}

/// Ticks per second of the `time` CSR, or `None` if the tree did not say.
///
/// Optional on purpose. It is required by the binding, but a caller that needs a
/// bounded wait must still work on a tree that omits it — so the answer is
/// `Option`, and the caller decides what to do without a clock rather than being
/// handed a fabricated frequency.
pub fn timebase_hz() -> Option<usize> {
    table::get().and_then(|t| t.timebase_hz)
}

/// Every MMIO window the device tree describes.
///
/// The **single** answer to "where is device memory". Anything that needs to map,
/// protect or enumerate it reads the list from here rather than deciding on a range
/// of its own.
///
/// A genuine walk of the tree, not a fixed list of the devices this kernel happens to
/// drive. That distinction is the point: an earlier version returned only UART, PLIC
/// and CLINT while claiming to be complete, which left a future virtio driver with no
/// way to find its window here — and the path of least resistance would have been its
/// own base constant, recreating exactly the split-brain this exists to prevent.
///
/// A window appearing here says the *device* exists, not that supervisor mode may
/// touch it: OpenSBI's PMP is a separate layer and denies S-mode access to the CLINT
/// on QEMU virt.
///
/// Empty before the tree has been parsed.
pub fn mmio_regions() -> &'static [PhysRegion] {
    table::get().map(|t| t.mmio.as_slice()).unwrap_or(&[])
}

/// Every RAM range that exists but is not the kernel's to hand out.
///
/// The frame allocator would otherwise vend all of it. Four sources, because the
/// previous boot stage has four different ways of leaving something behind and
/// honouring only some of them is indistinguishable from honouring none:
///
/// 1. **`/reserved-memory` nodes** — firmware carve-outs. OpenSBI adds its own here
///    (`mmode_resv0`/`1`) and its PMP then denies supervisor access.
/// 2. **The FDT memory reservation block** (header `off_mem_rsvmap`) — the *other*,
///    older spec mechanism for exactly the same purpose. Empty on QEMU virt with
///    OpenSBI, but mandated by the spec and used by U-Boot and coreboot, so reading
///    only `/reserved-memory` honours half the standard.
/// 3. **The initrd**, from `/chosen`'s `linux,initrd-start` / `linux,initrd-end`.
///    QEMU puts a 32 MiB one at `0x84200000` — squarely inside the pool.
/// 4. **The blob itself**, which lives in ordinary RAM near the top of it.
///
/// Two sources describing the same range is expected, not malformed: `memory::frame`
/// owns disjointness, because its outward page rounding is what destroys it. Merging
/// here would lose the names, and the names are how a later reclaim finds the initrd.
///
/// Empty before the tree has been parsed.
pub fn foreign_ram() -> &'static [PhysRegion] {
    table::get().map(|t| t.foreign.as_slice()).unwrap_or(&[])
}

/// Every hart id the machine reports, from `/cpus/cpu@N`'s `reg`.
///
/// A list to iterate, never a range to count, and never a source of array indices:
/// these ids need not be `0..n`. See [`crate::memory::stack`] for why that matters
/// and what ignoring it cost.
///
/// Harts whose node is not `status = "okay"` are absent: a disabled core cannot be
/// started, and allocating it a 64 KiB stack would be waste at best.
///
/// Empty before the tree has been parsed.
pub fn hart_ids() -> &'static [usize] {
    table::get().map(|t| t.hart_ids.as_slice()).unwrap_or(&[])
}
