# disable generation of compressed instructions.
.option norvc


.equ XLENB, 8

.macro LOAD_SP a1, a2
    ld \a1, \a2*XLENB(sp)
.endm

.macro STORE_SP a1, a2
    sd \a1, \a2*XLENB(sp)
.endm

.macro STORE_ALL
        # 中断可能来自用户态（U-Mode），也可能来自内核态（S-Mode）。
        # 如果是用户态中断，那么此时的栈指针 sp 指向的是用户栈；如果是内核态中断，那么 sp 指向的是内核栈。
        # 我们规定：当 CPU 处于 U-Mode 时，sscratch 保存内核栈地址；处于 S-Mode 时，sscratch 为 0 。

        # 交换 sp 和 sscratch 寄存器
        csrrw sp, sscratch, sp
        # 判断 sp（也就是交换前的 sscratch）是否为0
        # 如果非0，说明是用户态中断，由于 sscratch 保存的是内核栈地址
        # 此时 sp 已经指向内核栈，直接跳转到 trap_from_user 保存寄存器
        bnez sp, save_registers

    trap_from_kernel:
        # 如果是内核态中断，则继续使用内核栈即可。所以交换回去
        csrr sp, sscratch
    
    save_registers:
        # 为 TrapFrame 预留空间
        addi sp, sp, -34 * XLENB

        # save general registers except sp(x2)
        # 因为现在的 sp 指向中断处理时需要用到的 sp 而不是中断来源处的 sp
        # 我们将在稍后存储真正的 sp
        .set n, 0
        .rept 32
            STORE_SP x%n, %n
            .set n, n+1
        .endr
        csrr t0, sstatus
        STORE_SP t0, 32
        csrr t1, sepc
        STORE_SP t1, 33

        # 从 sscratch 中换回 sp 到 s0; 按照规定，进入内核态后 sscratch 应为 0
        csrrw s0, sscratch, x0
        # 保存 sp
        STORE_SP s0, 2

.endmacro

.macro RESTORE_ALL
        # 首先根据 sstatus 寄存器中的 SPP 位，判断是回到用户态还是内核态。
        # 如果是回到用户态，根据规定需要设置 sscratch 为内核栈
        csrr s0, sstatus # s0 = sstatus
        csrr s1, sepc # s1 = sepc
        andi s2, s0, 1 << 8     # sstatus.SPP = 1?
        bnez s2, restore_registers     # s0 = back to kernel?
    back_to_user:
        # 如果是回到用户态，根据规定需要还原当前 sp 到 sscratch
        addi s0, sp, 32*XLENB
        csrw sscratch, s0         # sscratch = kernel-sp
    restore_registers:
        LOAD_SP t1, 33
        csrw sepc, t1
        LOAD_SP t0, 32
        csrw sstatus, t0

        LOAD_SP x0, 0
        LOAD_SP x1, 0
        .set n, 3
        .rept 31
            LOAD_SP x%n, %n
            .set n, n+1
        .endr

        # release TrapContext on kernel stack
        addi sp, sp, 34 * XLENB
        # 目前 sp 指向内核栈，我们将其存回 sscratch
        csrrw sp, sscratch, sp

        # restore sp last
        LOAD_SP sp, 2
.endmacro


# ---

.section .text
.global trap_entry
.balign 4
trap_entry:
    STORE_ALL

    mv a0, sp
    jal trap_handler

trap_ret:
    RESTORE_ALL

    sret




