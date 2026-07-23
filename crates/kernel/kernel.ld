/* https://sourceware.org/binutils/docs/ld/Scripts.html */

OUTPUT_ARCH( "riscv" )

/*
stores the address of _start as the "entry point"
(or where CPU instructions should start executing.
*/
ENTRY( _boot )

/* The kernel is linked to run from the base of RAM. This base is the one
unavoidable compile-time hardware fact for a fixed-address (non-relocatable)
kernel: it is *where we are loaded*, not how much RAM exists.

There is deliberately NO MEMORY region and NO length/size here. A region would
force a LENGTH, and any length is a fiction: the linker cannot know the real RAM
top — that is the RAM extent the device tree reports at runtime (see
device_tree.rs). So instead of `>ram` with a bogus bound, we simply flow the
sections from the location counter starting at this base. The kernel image is a
few MiB; its usable top is enforced at runtime, not here. */
_memory_start = 0x8000_0000;

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
    /* Flow everything from the load base; VMA == LMA (identity loaded). */
    . = _memory_start;

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
    /* No MEMORY region: sections flow from the location counter, so VMA == LMA
       (each section is loaded at its physical address). ":text" assigns this
       section to the text program header declared in PHDRS. */
    } :text

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

        /* in case rodata section is empty, ensure it occupys at least one page */
        . = . + 4096;

        PROVIDE(_rodata_end = .);
    } :text

    .data : {
        . = ALIGN(4096);
        PROVIDE(_data_start = .);

        *(.sdata .sdata.*) 
        . = ALIGN(16);
        *(.data .data.*)

        /* in case data section is empty, ensure it occupys at least one page */
        . = . + 4096;

        PROVIDE(_data_end = .);
    } :data

    .bss : {
        . = ALIGN(4096);
        PROVIDE(_bss_start = .);

        *(.sbss .sbss.*)
        . = ALIGN(16);
        *(.bss .bss.*)

        /* in case bss section is empty, ensure it occupys at least one page */
        . = . + 4096;

        PROVIDE(_bss_end = .);
    } :bss


    /*
    the stack grows from higher memory to lower memory (bottom to top).
    we set the stack at the very bottom of its allocated slot.
    */
    /* our kernel stack starts at the end of the bss segment (_bss_end). */
    . = ALIGN(4096);
    PROVIDE(_kernel_stack_start = .);
    /* we're allocating 0x80000 bytes (512 KiB) to our kernel stack. */
    . = . + 0x100000;
    PROVIDE(_kernel_stack_end = .);
    

    /* our heap starts right after the kernel stack. Its END is not a linker
    symbol: the heap runs up to the RAM top discovered from the device tree at
    runtime (see memory::init), not to a compile-time LENGTH. */
    . = ALIGN(4096);
    PROVIDE(_heap_start = _kernel_stack_end + 4096);
}