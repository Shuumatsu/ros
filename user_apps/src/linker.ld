OUTPUT_ARCH(riscv)
ENTRY(_start)

BASE_ADDRESS = 0;

SECTIONS
{
    . = BASE_ADDRESS;

    .text : {
    PROVIDE(_text_start = .);

        *(.text.entry)
        *(.text .text.*)

        . = ALIGN(4K);
        PROVIDE(_text_end = .);
    }


    .rodata : {
        PROVIDE(_rodata_start = .);

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

        /* in case bss section is empty, ensure it occupys at least one page */
        . = . + 4096;
        . = ALIGN(4K);

        PROVIDE(_bss_end = .);
    }

    /DISCARD/ : {
        *(.eh_frame)
    }
}

