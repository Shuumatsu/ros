//! Kernel memory layout symbols from the linker script.

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
    kernel_stack_start => _kernel_stack_start,
    kernel_stack_end   => _kernel_stack_end,
    heap_start         => _heap_start,
);

// NOTE: only *address* symbols belong here. A small absolute linker symbol (a size
// like 4096) cannot be read this way — the reference is PC-relative and the value
// is nowhere near the code, so it fails to link with `R_RISCV_PCREL_HI20 out of
// range`. Sizes therefore live in Rust and are derived from the addresses above;
// see `memory::stack`.
