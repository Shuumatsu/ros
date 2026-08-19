/* Where `hello` is linked to run.

   The kernel's loader reads the program headers, so this file does not have to put anything at a
   particular offset — it only has to pick a base and keep the pieces in separate pages, which is
   what gives each of them program-header rights of its own. */

OUTPUT_ARCH( "riscv" )
ENTRY( _start )

SECTIONS {
    /* Comfortably clear of the null page, and unmistakably a user address. */
    . = 0x10000;

    .text : {
        *(.text .text.*)
    }

    /* Page aligned, so the linker gives it a segment of its own rather than folding it into an
       executable one. Same below. */
    . = ALIGN(4096);
    .rodata : {
        *(.srodata .srodata.*)
        *(.rodata .rodata.*)
    }

    . = ALIGN(4096);
    .data : {
        *(.sdata .sdata.*)
        *(.data .data.*)
    }

    .bss : {
        *(.sbss .sbss.*)
        *(.bss .bss.*)
    }

    /* Unwinding tables for a program that aborts. */
    /DISCARD/ : { *(.eh_frame*) }
}
