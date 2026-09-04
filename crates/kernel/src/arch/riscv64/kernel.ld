OUTPUT_ARCH( "riscv" )

/* The assertions below also pin the flat Image entry to this symbol. */
ENTRY( _start )

/* The kernel is loaded at 0x8020_0000 and linked at PA + _va_offset. Section LMAs
   use AT(VMA - _va_offset). RAM extent is discovered from the device tree at runtime.
   Boot-time checks verify `_page_size` and `_va_offset` against their Rust constants. */
_page_size      = 4096;                     /* architectural: Sv39 base page */
_dram_base      = 0x80000000;               /* RAM base: where the loader puts us */
_va_offset      = 0xffffffc000000000;       /* bottom of the Sv39 high half */
_text_offset    = 0x200000;                 /* SBI reserves the first 2 MiB */

_phys_base    = _dram_base + _text_offset;      /* 0x80200000: where we load */
_memory_start = _phys_base + _va_offset;        /* 0xffffffc080200000: virtual base */

/* Separate PT_LOAD segments permit distinct text, data, and BSS mappings. */
PHDRS {
    text PT_LOAD;
    data PT_LOAD;
    bss PT_LOAD;
}

/* Named sections are page-aligned for independent page-table permissions. */
SECTIONS {
    /* VMA is high; each AT expression supplies the physical load address. */
    . = _memory_start;

    .text : AT(ADDR(.text) - _va_offset) {
        PROVIDE(_text_start = .);

        /* The header must be first and its entry within `j` range. KEEP retains boot code
           under --gc-sections; the wildcard must remain last to include all ordinary text. */
        KEEP(*(.text.init.header))
        KEEP(*(.text.init.entry))
        KEEP(*(.text.init.trap))
        *(.text .text.*)

        PROVIDE(_text_end = .);
    } :text

    .rodata : AT(ADDR(.rodata) - _va_offset) {
        . = ALIGN(_page_size);
        PROVIDE(_rodata_start = .);

        /* Keep small read-only data within short `gp`-relative range. */
        *(.srodata .srodata.*)
        . = ALIGN(16);
        *(.rodata .rodata.*)

        /* Explicit placement keeps unwind data mapped and available to GDB. */
        . = ALIGN(8);
        *(.eh_frame .eh_frame.*)
        *(.eh_frame_hdr)

        . = . + _page_size;

        PROVIDE(_rodata_end = .);
    } :text

    .data : AT(ADDR(.data) - _va_offset) {
        . = ALIGN(_page_size);
        PROVIDE(_data_start = .);

        PROVIDE(__global_pointer$ = . + 0x800);
        *(.sdata .sdata.*) 
        . = ALIGN(16);
        *(.data .data.*)

        . = . + _page_size;

        PROVIDE(_data_end = .);
    } :data

    .bss : AT(ADDR(.bss) - _va_offset) {
        . = ALIGN(_page_size);
        PROVIDE(_bss_start = .);

        *(.sbss .sbss.*)
        . = ALIGN(16);
        *(.bss .bss.*)

        . = . + _page_size;

        /* `clear_bss` stores `usize` words, so the span must be a multiple of eight. */
        . = ALIGN(8);
        PROVIDE(_bss_end = .);
    } :bss


    /* The boot stack's Rust static defines its size. NOLOAD reserves no image bytes;
       secondary stacks are allocated at runtime. */
    . = ALIGN(_page_size);
    .boot_stack (NOLOAD) : {
        PROVIDE(_boot_stack_start = .);
        /* The stack has no Rust reference and must survive --gc-sections. */
        KEEP(*(.boot_stack))
        PROVIDE(_boot_stack_end = .);
    } :bss


    /* Derive image extent from the location counter so orphan sections cannot fall outside
       the reserved kernel range. The trailing page guards the boot stack. */
    . = ALIGN(_page_size);
    _image_end = .;
    . = . + _page_size;              /* guard page above the boot stack */
    PROVIDE(_kernel_top = .);
}

/* Page-sized gaps detect orphan sections that the kernel page table would not map. */
ASSERT(_start == _memory_start, "_start must be the first byte of the Image")
ASSERT(_image_header_end - _start == 64, "RISC-V Image header must be 64 bytes")
ASSERT((trap_park & 3) == 0, "the boot park vector must be 4-byte aligned: stvec reads its low two bits as a mode")
ASSERT(_rodata_start - _text_end   < _page_size, "orphan section between .text and .rodata")
ASSERT(_data_start   - _rodata_end < _page_size, "orphan section between .rodata and .data")
ASSERT(_bss_start    - _data_end   < _page_size, "orphan section between .data and .bss")
ASSERT(_boot_stack_start - _bss_end < _page_size, "orphan section between .bss and .boot_stack")
ASSERT(_image_end == _boot_stack_end, "orphan section after .boot_stack")
ASSERT((_boot_stack_end & 15) == 0, "boot stack top must be 16-byte aligned")

/* Image-header reservation includes BSS, boot stack, and its guard page. */
_image_size = _kernel_top - _memory_start;