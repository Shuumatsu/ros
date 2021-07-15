OUTPUT_ARCH(riscv)
ENTRY(_start)

MEMORY {
    /* we have "ram" that contains a section that is 
    'w' (writeable), 'x' (executable), and 'a' (allocatable). */

    /* qemu is set to have 128M memory starting from 0x80000000 */
    ram  (wxa) : ORIGIN = 0x80200000, LENGTH = 126M
}

PROVIDE(_memory_start = ORIGIN(ram));
PROVIDE(_memory_end = ORIGIN(ram) + LENGTH(ram));

SECTIONS
{
    . = ORIGIN(ram);
    PROVIDE(_kernel_start = .);

    .text : {
        PROVIDE(_text_start = .);

        *(.text.entry)
        *(.text .text.*)

        . = ALIGN(4K);
        PROVIDE(_text_end = .);
    }


    .rodata : {
        PROVIDE(_rodata_start = .);

        /* srodata and rodata are essentially the same thing. */
        /* However, compilers usually use the srodata sections for shorter, quicker loading sections.  */
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)

        /* in case rodata section is empty, ensure it occupys at least one page */
        . = . + 4096;
        . = ALIGN(4K);

        PROVIDE(_rodata_end = .);
    }

    .data : {
        PROVIDE(_data_start = .);

        *(.data .data.*)
        *(.sdata .sdata.*)

        /* in case rodata section is empty, ensure it occupys at least one page */
        . = . + 4096;
        . = ALIGN(4K);

        PROVIDE(_data_end = .);
    }

    .bss : {
        . = ALIGN(4K);
        PROVIDE(_bss_start = .);

        *(.bss .bss.*)
        *(.sbss .sbss.*)

        PROVIDE(_bss_end = .);

        . = ALIGN(4K);
        /* we put stack space inside bss */
        PROVIDE(_kernel_stack_start = .);
        /* we're allocating 0x80000 bytes (512 KiB) to our kernel stack. */
        . = . + 0x100000;
        PROVIDE(_kernel_stack_end = .);
    }

    PROVIDE(_kernel_end = .);

    . = ALIGN(4096);
    PROVIDE(_heap_start = .);
    PROVIDE(_heap_end = _memory_end);
}

