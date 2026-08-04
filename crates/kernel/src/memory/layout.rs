//! Kernel memory layout symbols from the linker script.

use paging::sv39::PAGE_SIZE;
use paging::{MemoryAddr, VirtualAddr};

/// Declares an extern linker symbol and creates an accessor function.
///
/// The kernel is linked high, so every one of these is a *virtual* address and is
/// typed as one at the source. That is what stops one being handed to something
/// expecting a physical address without a visible [`super::virt_to_phys`] in between.
///
/// The raw symbols are declared once, here, and re-exported: the boot entry needs
/// some of them as `sym` operands, where a typed accessor is no use, and a second
/// `extern` block naming the same symbols would be a second place for the linker
/// script's spelling to be wrong.
macro_rules! linker_symbol {
    ($($fn_name:ident => $sym_name:ident),* $(,)?) => {
        unsafe extern "C" {
            $(
                #[doc = concat!("Raw `", stringify!($sym_name), "`. Prefer [`", stringify!($fn_name), "`].")]
                pub(crate) static $sym_name: u8;
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
    heap_start         => _heap_start,
);

// NOTE: only *address* symbols belong here. A small absolute linker symbol (a size
// like 4096) cannot be read this way — the reference is PC-relative and the value
// is nowhere near the code, so it fails to link with `R_RISCV_PCREL_HI20 out of
// range`. Sizes therefore live in Rust and are derived from the addresses above;
// see `memory::stack`.

/// Zero `.bss`, putting the statics into the state Rust compiled against.
///
/// The loader copies only the sections that have bytes in the image; `.bss` has
/// none, so until this runs every static holds whatever the previous occupant of
/// that RAM left. First thing on the boot hart, and nowhere else — a secondary
/// arrives long after, into statics the boot hart is already using.
///
/// # Safety
///
/// Call exactly once, before anything reads or writes a static.
pub unsafe fn clear_bss() {
    let start = bss_start().bits();
    let end = bss_end().bits();

    // Volatile, and not `write_bytes`. To the compiler `_bss_start` is a lone
    // one-byte object and the statics being zeroed here are unrelated globals it
    // can see are never read beforehand, which makes a plain store one it is free
    // to narrow, sink past the first reader, or drop entirely.
    //
    // `kernel.ld` aligns both ends to a `usize`, so the last store lands inside.
    let mut addr = start;
    while addr < end {
        unsafe { (addr as *mut usize).write_volatile(0) };
        addr += size_of::<usize>();
    }
}

/// Assert the linker script's view of the layout matches Rust's.
///
/// `kernel.ld` has its own `_page_size` because it cannot read
/// [`paging::sv39::PAGE_SIZE`] (see the note above). The duplicate is verified rather
/// than read: the gap the linker *built* between the boot stack and the heap is one
/// page exactly when the two agree. A mismatch otherwise surfaces as unaligned
/// sections and guard pages drifted out of position.
///
/// Call once, before anything derives an address from these symbols.
pub fn check() {
    let guard = heap_start().sub_addr(boot_stack_end());
    assert_eq!(
        guard, PAGE_SIZE,
        "kernel.ld padded {guard:#x} bytes between the boot stack and the heap, but Rust's \
         PAGE_SIZE is {PAGE_SIZE:#x}; the linker's _page_size and PAGE_SIZE disagree"
    );

    // Every section the kernel maps separately must start on a page, or its region
    // would either overlap its neighbour or need rounding that swallows a guard.
    for (name, addr) in [
        ("_memory_start", memory_start()),
        ("_rodata_start", rodata_start()),
        ("_data_start", data_start()),
        ("_bss_start", bss_start()),
        ("_boot_stack_start", boot_stack_start()),
        ("_heap_start", heap_start()),
    ] {
        assert!(addr.is_aligned(PAGE_SIZE), "{name} = {addr:#x} is not page aligned");
    }
}
