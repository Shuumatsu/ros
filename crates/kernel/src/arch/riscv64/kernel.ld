/* https://sourceware.org/binutils/docs/ld/Scripts.html */

OUTPUT_ARCH( "riscv" )

/* The ELF entry point. The flat Image is entered at offset zero instead; the ASSERTs at
the bottom pin the two to the same instruction. */
ENTRY( _start )

/* Higher-half kernel: linked to *run* at high virtual addresses, but *loaded*
at physical 0x8020_0000 (RAM base + text_offset; the SBI firmware occupies the
first 2 MiB). So VMA (where symbols resolve) and LMA (where bytes are loaded)
differ by a fixed offset: VA = PA + _va_offset. Each section sets its LMA with
AT(ADDR(.x) - _va_offset) below; the architecture boot entry installs the page
table and jumps high before ordinary Rust runs.

There is no MEMORY region / LENGTH: the RAM top is a runtime fact from the device
tree (see `device_tree`), not a compile-time constant. */
/* SINGLE SOURCE OF TRUTH for the kernel's memory layout. Four genuine inputs —
   one architectural (`_page_size`) and three platform facts; everything else is
   derived, so there are no composite magic numbers to keep in sync.
   `_text_offset` is consumed by the Image header.

   `_va_offset` is the base of the kernel's DIRECT MAP: VA = PA + _va_offset holds
   for all of physical memory, not just for the kernel image, so it is a pure
   constant and NOT skewed by _dram_base. That is what lets Rust express the same
   value as a `const` (memory::direct_map::VA_OFFSET) and translate at compile
   time. It is the one value the linker and Rust cannot share directly, so the boot
   entry compares them — the VMA the linker wrote into the image against the address
   the code is running at — and parks instead of jumping high if they disagree.

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

/* Three program headers, one per kind of content:
       text - CPU instructions and read-only data
       data - initialized globals
       bss  - uninitialized globals (cleared by memory::layout::clear_bss)

   PT_LOAD marks each as a segment the loader brings into memory. */
PHDRS {
    text PT_LOAD;
    data PT_LOAD;
    bss PT_LOAD;
}

/* The image, section by section, in load order. Rust emits CPU instructions into
   .text, global constants into .rodata, initialized globals into .data and
   uninitialized ones into .bss; each gets a page-aligned range here so
   memory::kernel_table can give it its own rights. */
SECTIONS {
    /* VMA starts at the high virtual base; each section's LMA is AT(VMA - offset)
       = its physical load address. */
    . = _memory_start;

    .text : AT(ADDR(.text) - _va_offset) {
        PROVIDE(_text_start = .);

        /* The boot stage, in this order because the Image header must be the image's
           first byte and the entry it branches to must be within a `j` of it. KEEP states
           that the stage is required whatever --gc-sections concludes about it. Each
           carries the alignment its own code asks for.

           The wildcard last, and it must stay: anything left unmatched is placed after
           _text_end, outside every range kernel_table maps. */
        KEEP(*(.text.init.header))
        KEEP(*(.text.init.entry))
        KEEP(*(.text.init.trap))
        *(.text .text.*)

        PROVIDE(_text_end = .);
    } :text

    .rodata : AT(ADDR(.rodata) - _va_offset) {
        . = ALIGN(_page_size);
        PROVIDE(_rodata_start = .);

        /* .srodata holds the small read-only data the compiler wants reachable by a
           short `gp`-relative load, so it goes first, nearest __global_pointer$. */
        *(.srodata .srodata.*)
        . = ALIGN(16);
        *(.rodata .rodata.*)

        /* .eh_frame must be named here, not left to orphan placement: unnamed, the
           linker puts it after _rodata_end, inside the loaded image but outside
           every range kernel_table maps. Kept rather than discarded because
           scripts/run.sh exposes a gdb stub and gdb unwinds through it; it is
           read-only data, so .rodata is where it belongs. */
        . = ALIGN(8);
        *(.eh_frame .eh_frame.*)
        *(.eh_frame_hdr)

        /* in case rodata section is empty, ensure it occupies at least one page */
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

        /* in case data section is empty, ensure it occupies at least one page */
        . = . + _page_size;

        PROVIDE(_data_end = .);
    } :data

    .bss : AT(ADDR(.bss) - _va_offset) {
        . = ALIGN(_page_size);
        PROVIDE(_bss_start = .);

        *(.sbss .sbss.*)
        . = ALIGN(16);
        *(.bss .bss.*)

        /* in case bss section is empty, ensure it occupies at least one page */
        . = . + _page_size;

        /* memory::layout::clear_bss zeroes this span one `usize` at a time, stopping
           when the cursor reaches _bss_end, so a span that is not a multiple of 8
           makes the last store write up to 7 bytes past it. */
        . = ALIGN(8);
        PROVIDE(_bss_end = .);
    } :bss


    /* The BOOT HART's kernel stack, and only that one — hence no array and no index.
       Secondary harts get theirs from the frame allocator at runtime, because by the
       time one starts there is an allocator to ask, and because how many harts exist
       is a property of the machine rather than of this file. `memory::stack`'s module
       docs own the rest, including why nothing is keyed by hart id.

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


    /* The top of everything the kernel occupies, guard page included. NOT the bottom
    of free RAM: the frame allocator manages the whole RAM bank and withholds what is
    spoken for by address, this span among it (see memory::init), so what lies below
    _memory_start is free RAM as much as what lies above here is.

    Both symbols below come from the location counter, NOT from _boot_stack_end.
    Writing `_boot_stack_end + _page_size` would restate "the image ends at the boot
    stack" as arithmetic, and the linker does place sections this script omits. A
    NOBITS orphan ranked after .boot_stack (it and .bss are both NOBITS+WA) would put
    _kernel_top inside it — and memory::init withholds only up to _kernel_top, so the
    allocator would vend frames over live kernel data. `.` cannot drift from where the
    linker actually is. */
    . = ALIGN(_page_size);
    _image_end = .;
    . = . + _page_size;              /* guard page above the boot stack */
    PROVIDE(_kernel_top = .);
}

/* No section may appear after .boot_stack, and none between the five named above.
   Each named section opens with ALIGN(_page_size), so a normal inter-section gap is
   under a page; an orphan squeezed in makes it at least a page bigger.

   This is what verifies the linker's own output. _va_offset, _page_size and the
   .boot_stack span are all checked at boot; which sections were emitted is checked
   here, at link time. A tripped assert means: name the new section explicitly and
   decide whether kernel_table should map it. */
ASSERT(_start == _memory_start, "_start must be the first byte of the Image")
ASSERT(_image_header_end - _start == 64, "RISC-V Image header must be 64 bytes")
ASSERT((_trap_vector & 3) == 0, "the early trap vector must be 4-byte aligned: stvec reads its low two bits as a mode")
ASSERT(_rodata_start - _text_end   < _page_size, "orphan section between .text and .rodata")
ASSERT(_data_start   - _rodata_end < _page_size, "orphan section between .rodata and .data")
ASSERT(_bss_start    - _data_end   < _page_size, "orphan section between .data and .bss")
ASSERT(_boot_stack_start - _bss_end < _page_size, "orphan section between .bss and .boot_stack")
ASSERT(_image_end == _boot_stack_end, "orphan section after .boot_stack")
ASSERT((_boot_stack_end & 15) == 0, "boot stack top must be 16-byte aligned")

/* Effective image size for the RISC-V Image header: everything the loader must
   reserve for the kernel — loaded image, BSS, boot stack and its trailing guard.
   Secondary hart stacks are absent on purpose: they come from the frame allocator,
   out of RAM the loader has already been told about. This is an absolute symbol (a
   constant difference), so the Image header emits its value rather than an
   address. */
_image_size = _kernel_top - _memory_start;