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

        /* .eh_frame is named HERE, deliberately, and not left to orphan placement.
           Unnamed, the linker put it immediately after _rodata_end — inside the
           loaded image but outside every range kernel_table maps, so its last 248
           bytes lived on an unmapped page. Harmless only because panic = "abort"
           means nothing ever reads it. It is kept rather than discarded because
           scripts/run.sh exposes a gdb stub and gdb unwinds through .eh_frame; it
           is read-only data, so .rodata is where it belongs. */
        . = ALIGN(8);
        *(.eh_frame .eh_frame.*)
        *(.eh_frame_hdr)

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

        /* boot.S zeroes this span with `sd`, stepping 8 bytes at a time and
           stopping on `bltu`, so a span that is not a multiple of 8 makes the last
           store write up to 7 bytes past _bss_end. It has always happened to be
           8-aligned; nothing made it so. */
        . = ALIGN(8);
        PROVIDE(_bss_end = .);
    } :bss


    /* The BOOT HART's kernel stack, and only that one. Secondary harts get theirs
       from the frame allocator at runtime, because by the time one starts there is
       an allocator to ask — and because the number of harts is a property of the
       machine, not of this file. There is deliberately no per-hart array here any
       more; memory::stack's module docs own that story, including why indexing one
       by hart id was wrong.

       NO SIZE APPEARS HERE, only placement. `memory::stack` declares the area as a
       single `.boot_stack` static and this section takes the size from it, so the
       guard size and the stack size are stated exactly once, in Rust, where they can
       be read and checked. A byte count written here would be a composite magic
       number encoding both, verifiable by nothing.
       `memory::stack::check_layout` asserts at boot that the section landed with the
       span the geometry expects.

       NOLOAD because this is an uninitialised stack: reserve the address space, but
       put no zeros in the image. */
    . = ALIGN(_page_size);
    .boot_stack (NOLOAD) : {
        PROVIDE(_boot_stack_start = .);
        /* KEEP: nothing in Rust references the static, so --gc-sections would
           otherwise discard the kernel's stack. */
        KEEP(*(.boot_stack))
        PROVIDE(_boot_stack_end = .);
    } :bss


    /* our heap starts right after the boot stack. Its END is not a linker
    symbol: the heap runs up to the RAM top discovered from the device tree at
    runtime (see memory::init), not to a compile-time LENGTH.

    Both symbols below come from the location counter, NOT from _boot_stack_end.
    Writing `_boot_stack_end + _page_size` restated "the image ends at the boot
    stack" as arithmetic, and .eh_frame above is proof the linker places sections
    this script does not mention. A NOBITS orphan ranked after .boot_stack (it and
    .bss are both NOBITS+WA, so this is plausible) would have put _heap_start
    inside it — and memory::init hands virt_to_phys(_heap_start) to frame::init as
    the bottom of free RAM, so the allocator would vend frames over live kernel
    data. `.` cannot drift from where the linker actually is. */
    . = ALIGN(_page_size);
    _image_end = .;
    . = . + _page_size;              /* guard page below the heap */
    PROVIDE(_heap_start = .);
}

/* No section may appear after .boot_stack, and none between the five named above.
   Each named section opens with ALIGN(_page_size), so the gap between one
   section's end and the next one's start is normally less than a page; an orphan
   squeezed in makes it at least a page bigger. That is the whole check. It is
   cheap and it is exact, and without it the linker's own output was the one input
   to this kernel that nothing verified — while _va_offset, _page_size and the
   .boot_stack span all get checked at boot. A tripped assert here means: name the
   new section explicitly and decide whether kernel_table should map it. */
ASSERT(_rodata_start - _text_end   < _page_size, "orphan section between .text and .rodata")
ASSERT(_data_start   - _rodata_end < _page_size, "orphan section between .rodata and .data")
ASSERT(_bss_start    - _data_end   < _page_size, "orphan section between .data and .bss")
ASSERT(_boot_stack_start - _bss_end < _page_size, "orphan section between .bss and .boot_stack")
ASSERT(_image_end == _boot_stack_end, "orphan section after .boot_stack")

/* Effective image size for the RISC-V Image header: everything the loader must
   reserve for the kernel — loaded image + BSS + boot stack. Secondary hart
   stacks are absent on purpose: they come from the frame allocator, out of RAM
   the loader has already been told about. This is an absolute symbol (a constant
   difference), so `.8byte _image_size` in boot.S emits its value, not an
   address. */
_image_size = _image_end - _memory_start;