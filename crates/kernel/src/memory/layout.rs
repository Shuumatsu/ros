//! Kernel memory layout symbols from the linker script.

use paging::sv39::PAGE_SIZE;

/// Declares an extern linker symbol and creates an accessor function.
macro_rules! linker_symbol {
    ($($fn_name:ident => $sym_name:ident),* $(,)?) => {
        $(
            #[inline]
            pub fn $fn_name() -> usize {
                unsafe extern "C" {
                    static $sym_name: u8;
                }
                unsafe { &$sym_name as *const _ as usize }
            }
        )*
    };
}

linker_symbol!(
    text_start         => _text_start,
    text_end           => _text_end,
    global_pointer     => _global_pointer,
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
    // Not a layout bound but a linker symbol read the same way, so it uses the same
    // macro rather than a second hand-rolled `extern "C" { static … }` elsewhere.
    // `cpu::start_secondaries` passes its *physical* address to SBI: a hart starts
    // with translation off, and this is an entry point, not a callable function.
    secondary_entry    => _secondary_start,
);

// NOTE: only *address* symbols belong here. A small absolute linker symbol (a size
// like 4096) cannot be read this way — the reference is PC-relative and the value
// is nowhere near the code, so it fails to link with `R_RISCV_PCREL_HI20 out of
// range`. Sizes therefore live in Rust and are derived from the addresses above;
// see `memory::stack`.

/// Assert the linker script's view of the layout matches Rust's.
///
/// `kernel.ld` has its own `_page_size` because it cannot read
/// [`paging::sv39::PAGE_SIZE`] (see the note above). The duplicate is unavoidable,
/// so it is verified instead — not by reading the symbol, but by measuring something
/// the linker *built* with it: the gap it left between the boot stack and the heap is
/// exactly one page. If the two ever disagree, this catches it at boot rather than
/// letting sections land unaligned and the guard pages drift out of position.
///
/// Call once, before anything derives an address from these symbols.
pub fn check() {
    let guard = heap_start() - boot_stack_end();
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
        assert_eq!(addr % PAGE_SIZE, 0, "{name} = {addr:#x} is not page aligned");
    }
}
