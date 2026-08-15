//! The symbols `kernel.ld` defines, and the kernel's view of the layout they describe.
//!
//! Every name the linker script chooses lives here, `__global_pointer$` included.

use paging::sv39::PAGE_SIZE;
use paging::{MemoryAddr, VirtualAddr};

/// Declares every symbol `kernel.ld` defines, in one place.
///
/// One `extern` block, because a second would be a second place for the linker script's
/// spelling to be wrong. The raw statics are `pub(crate)` because the boot entry needs
/// some as `sym` operands.
///
/// `addresses` get an accessor as well: the kernel is linked high, so each is a *virtual*
/// address and is typed as one at the source. `raw` is for the rest — a symbol the kernel
/// only ever names, which stays untyped because arithmetic on it would mean nothing.
macro_rules! linker_symbol {
    (
        addresses { $($fn_name:ident => $sym_name:ident),* $(,)? }
        raw { $($(#[$raw_doc:meta])* $raw_name:ident $(as $link_name:literal)?),* $(,)? }
    ) => {
        unsafe extern "C" {
            $(
                #[doc = concat!("Raw `", stringify!($sym_name), "`. Prefer [`", stringify!($fn_name), "`].")]
                pub(crate) static $sym_name: u8;
            )*
            $(
                $(#[$raw_doc])*
                $(#[link_name = $link_name])?
                $(#[doc = concat!("Linked as `", $link_name, "`.")])?
                pub(crate) static $raw_name: u8;
            )*
        }
        $(
            #[inline]
            pub fn $fn_name() -> VirtualAddr {
                VirtualAddr::new(&raw const $sym_name as usize)
            }
        )*
    };
}

linker_symbol!(
    addresses {
        text_start         => _text_start,
        text_end           => _text_end,
        rodata_start       => _rodata_start,
        rodata_end         => _rodata_end,
        data_start         => _data_start,
        data_end           => _data_end,
        bss_start          => _bss_start,
        bss_end            => _bss_end,
        memory_start       => _memory_start,
        boot_stack_start   => _boot_stack_start,
        boot_stack_end     => _boot_stack_end,
        free_ram_start     => _free_ram_start,
    }
    raw {
        /// The anchor `gp` holds, so that global access can be relaxed to `gp`-relative.
        /// A register value; the boot entry names it only to load `gp` from it.
        GLOBAL_POINTER as "__global_pointer$",
    }
);

// Symbols only, never *values*. A small absolute linker symbol (a size like 4096) fails to
// link this way — the reference is PC-relative — so sizes live in Rust and are derived from
// the addresses above. The two `kernel.ld` does define, `_text_offset` and `_image_size`,
// are named in assembly instead; the Image header is their only reader.

/// Zero `.bss`, putting the statics into the state Rust compiled against.
///
/// The loader copies only sections with bytes in the image, and `.bss` has none, so until
/// this runs every static holds whatever the previous occupant left. Boot hart only: a
/// secondary arrives into statics the boot hart is already using.
///
/// # Safety
///
/// Call exactly once, before anything reads or writes a static.
pub unsafe fn clear_bss() {
    let start = bss_start().bits();
    let end = bss_end().bits();

    // Volatile, not `write_bytes`: to the compiler `_bss_start` is a lone one-byte object
    // and the statics zeroed here are globals it can see are never read beforehand, so a
    // plain store is one it may narrow, sink or drop. `kernel.ld` aligns both ends to a
    // `usize`, so the last store lands inside.
    let mut addr = start;
    while addr < end {
        unsafe { (addr as *mut usize).write_volatile(0) };
        addr += size_of::<usize>();
    }
}

/// Assert the linker script's view of the layout matches Rust's. Call once, before
/// anything derives an address from these symbols.
///
/// `kernel.ld` carries its own `_page_size`, so the duplicate is verified by measuring
/// what the linker built: the gap above the boot stack is one page exactly when the two
/// agree. A mismatch otherwise surfaces as guard pages drifted out of position.
pub fn check() {
    let guard = free_ram_start().sub_addr(boot_stack_end());
    assert_eq!(
        guard, PAGE_SIZE,
        "kernel.ld padded {guard:#x} bytes between the boot stack and free RAM, but Rust's \
         PAGE_SIZE is {PAGE_SIZE:#x}; the linker's _page_size and PAGE_SIZE disagree"
    );

    // Every separately mapped section must start on a page, or its region overlaps its
    // neighbour or needs rounding that swallows a guard.
    for (name, addr) in [
        ("_memory_start", memory_start()),
        ("_rodata_start", rodata_start()),
        ("_data_start", data_start()),
        ("_bss_start", bss_start()),
        ("_boot_stack_start", boot_stack_start()),
        ("_free_ram_start", free_ram_start()),
    ] {
        assert!(addr.is_aligned(PAGE_SIZE), "{name} = {addr:#x} is not page aligned");
    }
}

/// Print the kernel's static memory layout. The geometry *inside* the boot stack area is
/// [`super::stack`]'s to report.
pub fn report() {
    println!("kernel image layout:");
    println!("    load base:    {:#x}", memory_start());
    println!("    text:         {:#x}..{:#x}", text_start(), text_end());
    println!("    rodata:       {:#x}..{:#x}", rodata_start(), rodata_end());
    println!("    data:         {:#x}..{:#x}", data_start(), data_end());
    println!("    bss:          {:#x}..{:#x}", bss_start(), bss_end());
    println!("    boot stack:   {:#x}..{:#x}", boot_stack_start(), boot_stack_end());
    // Where the image stops and the frame allocator's territory begins. Not a heap
    // address: the kernel heap is frames taken from the pool (see `super::heap`).
    println!("    free RAM:     {:#x}..", free_ram_start());
}
