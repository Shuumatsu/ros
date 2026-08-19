/* Where `hello` is linked to run, and the shape the kernel's loader expects.

   A FLAT IMAGE, like the kernel's own: `llvm-objcopy -O binary` drops the ELF and leaves the
   bytes, so the loader maps the file at BASE_ADDRESS and enters it at its first byte. That is
   the whole loader — no program headers to walk — and it costs two properties this file has to
   guarantee: the entry is at offset zero, and every byte the program needs is *in* the file. */

OUTPUT_ARCH( "riscv" )
ENTRY( _start )

/* Comfortably clear of the null page, page aligned, and unmistakably a user address. */
BASE_ADDRESS = 0x10000;

SECTIONS {
    . = BASE_ADDRESS;

    .text : {
        /* `.text.entry` first, because a flat image is entered at its first byte and function
           placement is otherwise the linker's business. */
        KEEP(*(.text.entry))
        *(.text .text.*)
    }

    .rodata : {
        *(.srodata .srodata.*)
        *(.rodata .rodata.*)
    }

    .data : {
        *(.sdata .sdata.*)
        *(.data .data.*)
    }

    /* Named rather than left to orphan placement, so the ASSERT below can see it. */
    .bss : {
        *(.sbss .sbss.*)
        *(.bss .bss.*)
    }

    /* Unwinding tables for a program that aborts, and would otherwise land in the image. */
    /DISCARD/ : { *(.eh_frame*) }
}

ASSERT(_start == BASE_ADDRESS, "the entry must be the first byte of the flat image")
/* A flat image carries no zero-filled tail, so a `.bss` here is memory the program reads and
   the loader never wrote. Zero it in the loader, or do without the static. */
ASSERT(SIZEOF(.bss) == 0, "hello has a .bss, which a flat image does not carry")
