/* https://sourceware.org/binutils/docs/ld/Scripts.html */

OUTPUT_ARCH( "riscv" )

/*
stores the address of _start as the "entry point"
(or where CPU instructions should start executing.
*/
ENTRY( _start )

/* Higher-half kernel: linked to *run* at high virtual addresses, but *loaded*
at physical 0x8020_0000 (RAM base + text_offset; the SBI firmware occupies the
first 2 MiB). So VMA (where symbols resolve) and LMA (where bytes are loaded)
differ by a fixed offset: VA = PA + _va_offset. Each section sets its LMA with
AT(ADDR(.x) - _va_offset) below; boot.S installs the page table and jumps high
before Rust runs.

Still NO MEMORY region / LENGTH: the RAM top is a runtime fact from the device
tree, not a compile-time constant (see device_tree.rs). */
/* SINGLE SOURCE OF TRUTH for the kernel's memory layout. Four genuine inputs —
   one architectural (`_page_size`) and three platform facts; everything else is
   derived, so there are no composite magic numbers to keep in sync.
   `_text_offset` is consumed by the Image header in boot.S.

   `_va_offset` is the base of the kernel's DIRECT MAP: VA = PA + _va_offset holds
   for all of physical memory, not just for the kernel image, so it is a pure
   constant and NOT skewed by _dram_base. That is what lets Rust express the same
   value as a `const` (memory::direct_map::VA_OFFSET) and translate at compile
   time. It is the one value the linker and Rust cannot share directly; boot.S
   measures the real VMA-PMA skew at boot and direct_map::verify() asserts the two
   agree, so the duplicate cannot ship diverged.

   `_page_size` is named rather than repeated: the eight places below that align or
   pad by a page then say *why*. Rust cannot read it (a small absolute linker symbol
   is not reachable PC-relative), so it keeps its own `paging::sv39::PAGE_SIZE`, and
   `memory::layout::check()` pins the two together at boot by measuring a gap the
   linker built — the same duplicate-and-verify arrangement as `_va_offset`. */
_page_size      = 4096;                     /* architectural: Sv39 base page */
_dram_base      = 0x80000000;               /* RAM base: where the loader puts us */
_va_offset      = 0xffffffc000000000;       /* bottom of the Sv39 high half */
_text_offset    = 0x200000;                 /* SBI reserves the first 2 MiB */

_phys_base    = _dram_base + _text_offset;      /* 0x80200000: where we load */
_memory_start = _phys_base + _va_offset;        /* 0xffffffc080200000: virtual base */

/*
PHDRS is short for "program headers", which we specify three here:
    text - CPU instructions (executable sections)
    data - Global, initialized variables
    bss  - Global, uninitialized variables (all will be set to 0 by boot.S)

The command PT_LOAD tells the linker that these sections will be loaded
from the file into memory.
*/
PHDRS {
    text PT_LOAD;
    data PT_LOAD;
    bss PT_LOAD;
}

/*
organize the memory based on which section it is in. 
in C++ and Rust, 
    - CPU instructions go into text, 
    - global constants go into rodata, 
    - global initialized variables go into data,
    - global uninitialized variables go into bss.
*/
SECTIONS {
    /* VMA starts at the high virtual base; each section's LMA is AT(VMA - offset)
       = its physical load address. */
    . = _memory_start;

    .text : AT(ADDR(.text) - _va_offset) {
        PROVIDE(_text_start = .);
        /*
        We are going to layout all text sections here, starting with 
        .text.init. The asterisk in front of the parentheses means to match
        the .text.init section of ANY object file. Otherwise, we can specify
        which object file should contain the .text.init section, for example,
        boot.o(.text.init) would specifically put the .text.init section of
        our bootloader here.

        Because we might want to change the name of our files, we'll leave it
        with a *.

        Inside the parentheses is the name of the section. I created my own
        called .text.init to make 100% sure that the _start is put right at the
        beginning. The linker will lay this out in the order it receives it:

        .text.init first
        all .text sections next
        any .text.* sections last

        .text.* means to match anything after .text. If we didn't already specify
        .text.init, this would've matched here. The assembler and linker can place
        things in "special" text sections, so we match any we might come across here.
        */
        *(.text.init) *(.text .text.*)

        PROVIDE(_text_end = .);
    } :text

    .rodata : AT(ADDR(.rodata) - _va_offset) {
        . = ALIGN(_page_size);
        PROVIDE(_global_pointer = .);
        PROVIDE(_rodata_start = .);

        /*
        srodata and rodata are essentially the same thing. However, compilers usually use the
        srodata sections for shorter, quicker loading sections. 
        */
        *(.srodata .srodata.*)
        . = ALIGN(16);
        *(.rodata .rodata.*)

        /* in case rodata section is empty, ensure it occupys at least one page */
        . = . + _page_size;

        PROVIDE(_rodata_end = .);
    } :text

    .data : AT(ADDR(.data) - _va_offset) {
        . = ALIGN(_page_size);
        PROVIDE(_data_start = .);

        *(.sdata .sdata.*) 
        . = ALIGN(16);
        *(.data .data.*)

        /* in case data section is empty, ensure it occupys at least one page */
        . = . + _page_size;

        PROVIDE(_data_end = .);
    } :data

    .bss : AT(ADDR(.bss) - _va_offset) {
        . = ALIGN(_page_size);
        PROVIDE(_bss_start = .);

        *(.sbss .sbss.*)
        . = ALIGN(16);
        *(.bss .bss.*)

        /* in case bss section is empty, ensure it occupys at least one page */
        . = . + _page_size;

        PROVIDE(_bss_end = .);
    } :bss


    /* Per-hart kernel stacks. Stacks grow DOWN, so each hart's usable stack sits
       immediately above an unmapped guard page: an overflow walks off the bottom
       into the guard and faults, instead of silently eating whatever is below it
       (hart 0's neighbour is .bss; every other hart's is the previous hart's
       stack). The guards are holes because memory::kernel_table maps each stack
       as its own region and never the guards.

       Layout for hart h, with stride = guard + stack:
           guard  [start + stride*h,         start + stride*h + guard)
           stack  [start + stride*h + guard, start + stride*(h+1))
       which makes the stack top — and therefore sp — simply
           start + stride*(h+1)
       so boot.S needs only the stride.

       NO SIZE APPEARS HERE. `memory::stack` declares the whole area as one
       `.hart_stacks` static sized `STRIDE * MAX_HARTS`, and this section merely
       places it — so the size comes from the section itself and the guard size, the
       stack size and the hart count are stated exactly once, in Rust, where they can
       be read and checked.

       Writing the total by hand instead (`. = . + 0x110000`) is a composite magic
       number: it silently encodes all three values, nothing can verify it, and
       changing the stack size in Rust would leave it stale while quietly altering
       the hart count. `memory::stack::check_layout` asserts at boot that the section
       landed with the span the geometry expects.

       NOLOAD because these are uninitialised stacks: reserve the address space, but
       put no zeros in the image. */
    . = ALIGN(_page_size);
    .hart_stacks (NOLOAD) : {
        PROVIDE(_kernel_stack_start = .);
        /* KEEP: nothing in Rust references the static, so --gc-sections would
           otherwise discard the kernel's stacks. */
        KEEP(*(.hart_stacks))
        PROVIDE(_kernel_stack_end = .);
    } :bss
    

    /* our heap starts right after the kernel stack. Its END is not a linker
    symbol: the heap runs up to the RAM top discovered from the device tree at
    runtime (see memory::init), not to a compile-time LENGTH. */
    . = ALIGN(_page_size);
    PROVIDE(_heap_start = _kernel_stack_end + _page_size);
}

/* Effective image size for the RISC-V Image header: everything the loader must
   reserve for the kernel — loaded image + BSS + kernel stack. This is an
   absolute symbol (a constant difference), so `.8byte _image_size` in boot.S
   emits its value, not an address. */
_image_size = _kernel_stack_end - _memory_start;