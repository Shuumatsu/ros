# https://github.com/riscv/riscv-asm-manual/blob/master/riscv-asm.md

.section .text.entry

.globl _start
_start:
    # rustsbi-qemu set a0 = hartid and a1 = dtd

    # Any hardware threads (hart) that are not bootstrapping
    bnez    a0, 2f

    # Global, uninitialized variables get the value 0 since these are allocated in the BSS section. 
    # However, since we're the OS, we are responsible for making sure that memory is 0.
    la      a2, _bss_start
    la      a3, _bss_end
    bgeu    a2, a3, 2f
1:
    sd      zero, (a2)
    addi    a2, a2, 8
    bltu    a2, a3, 1b 

2:
    # set kernel stacks for each hart, and make sure they are 0
    # allocate 64kb stack for each hart
    la      sp, _kernel_stack_end
    # for a2 = 0; a2 < a0; a2 += 1
    li      a2, 0
    bgeu    a2, a0, 4f
3:
    li      a4, -65536
    #       sp -= 65536
    add     sp, sp, a4
    addi    a2, a2, 1
    # if a0 < a2 then goto 3b
    bltu    a2, a0, 3b


4:
.global _before_entry
_before_entry:
    call    rust_entry
