# disable generation of compressed instructions.
.option norvc

.section .text.init

.global _boot
_boot:
    # SATP should be zero, but let's make sure
    csrw    satp, zero
    
    # ".option push" is used to save all existing settings for options 
    # so that you can change something temporarily.
    .option push
    # .option norelax tells the linker to not relax AUIPC+ADDI to a gp relative reference (constant pool).
    .option norelax
        # Data that is globally declared in an application is allocated to memory. 
        # When referencing (loading or storing) this data that has been allocated to memory, 
        # the global pointer (gp) is provided to enable access independent of the allocation position 
        # (PID: Position Independent Data).
        # http://tool-support.renesas.com/autoupdate/support/onlinehelp/csp/V4.01.00/CS+.chm/Compiler-CCRH.chm/Output/ccrh08c0401y.html#:~:text=Data%20that%20is%20globally%20declared,PID%3A%20Position%20Independent%20Data).
        la        gp, _global_pointer
    # "option pop" later restores all the previous options without having to change each one back individually.
    .option pop

    # keep each CPU's hartid in its tp register
    csrr    tp, mhartid

    # Any hardware threads (hart) that are not bootstrapping
    csrr    t0, mhartid
    bnez    t0, 2f

    # Global, uninitialized variables get the value 0 since these are allocated in the BSS section. 
    # However, since we're the OS, we are responsible for making sure that memory is 0.
    la      a0, _bss_start
    la      a1, _bss_end
    bgeu    a0, a1, 2f
1:
    sd      zero, (a0)
    addi    a0, a0, 8
    bltu    a0, a1, 1b

2:    
    # stack grows from bottom to top, so we put the stack pointer to the very end of the stack range.
    la      sp, _kernel_stack_start

    # Allocate 64kb stack for each hart
    li      a0, 64*1024
	csrr    a1, mhartid
	addi    a1, a1, 1
    mul     a0, a0, a1
    add     sp, sp, a0

    call start
