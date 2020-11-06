/* https://sourceware.org/binutils/docs/ld/Scripts.html */

OUTPUT_ARCH( "riscv" )

/*
stores the address of _start as the "entry point"
(or where CPU instructions should start executing.
*/
ENTRY( _boot )

MEMORY {
    /* we have "ram" that contains a section that is 
    'w' (writeable), 'x' (executable), and 'a' (allocatable). */
    ram  (wxa) : ORIGIN = 0x80000000, LENGTH = 128M
}

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
    .text : {
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
    /*
    The portion after the right brace is in an odd format. However, this is telling the
    linker what memory portion to put it in. We labeled our RAM, ram, with the constraints
    that it is writeable, allocatable, and executable. The linker will make sure with this
    that we can do all of those things.

        >ram - This just tells the linker script to put this entire section (.text) into the
                ram region of memory. To my knowledge, the '>' does not mean "greater than". Instead,
            it is a symbol to let the linker know we want to put this in ram.

        AT>ram - This sets the LMA (load memory address) region to the same thing. LMA is the final
                    translation of a VMA (virtual memory address). With this linker script, we're loading
                everything into its physical location. We'll let the kernel copy and sort out the 
                virtual memory. That's why >ram and AT>ram are continually the same thing.

        :text  - This tells the linker script to put this into the :text program header. We've only
                    defined three: text, data, and bss. In this case, we're telling the linker script
                to go into the text section.
    */
    } >ram AT>ram :text

    .rodata : {
        . = ALIGN(4096);
        PROVIDE(_global_pointer = .);
        PROVIDE(_rodata_start = .);

        /*
        srodata and rodata are essentially the same thing. However, compilers usually use the
        srodata sections for shorter, quicker loading sections. 
        */
        *(.srodata .srodata.*)
        . = ALIGN(16);
        *(.rodata .rodata.*)

        PROVIDE(_rodata_end = .);
    } >ram AT>ram :text

    .data : {
        . = ALIGN(4096);
        PROVIDE(_data_start = .);

        *(.sdata .sdata.*) 
        . = ALIGN(16);
        *(.data .data.*)

        PROVIDE(_data_end = .);
    } >ram AT>ram :data

    .bss : {
        . = ALIGN(4096);
        PROVIDE(_bss_start = .);

        *(.sbss .sbss.*)
        . = ALIGN(16);
        *(.bss .bss.*)

        PROVIDE(_bss_end = .);
    } >ram AT>ram :bss


    /* _memory_start will be set to 0x8000_0000 here. We use ORIGIN(ram) so that it will take
    whatever we set the origin of ram to.  */
    PROVIDE(_memory_start = ORIGIN(ram));

    /*
    the stack grows from higher memory to lower memory (bottom to top).
    we set the stack at the very bottom of its allocated slot.
    */
    /* our kernel stack starts at the end of the bss segment (_bss_end). */
    PROVIDE(_kernel_stack_start = _bss_end);
    /* we're allocating 0x80000 bytes (512 KiB) to our kernel stack. */
    PROVIDE(_kernel_stack_end = _kernel_stack_start + 0x80000);
    PROVIDE(_memory_end = ORIGIN(ram) + LENGTH(ram));

    /* our heap starts right after the kernel stack */
    /* we don't align here because we let the kernel determine how it wants to do this. */
    PROVIDE(_heap_start = _kernel_stack_end);
    PROVIDE(_heap_size = _memory_end - _heap_start);
}