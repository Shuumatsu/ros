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
        addi sp, sp, -32 * XLENB

        # save general registers except sp(x2)
        # 因为现在的 sp 指向中断处理时需要用到的 sp 而不是中断来源处的 sp
        # 我们将在稍后存储真正的 sp
        STORE_SP x0, 0
        STORE_SP x1, 1
        STORE_SP x3, 3
        STORE_SP x4, 4
        STORE_SP x5, 5
        STORE_SP x6, 6
        STORE_SP x7, 7
        STORE_SP x8, 8
        STORE_SP x9, 9
        STORE_SP x10, 10
        STORE_SP x11, 11
        STORE_SP x12, 12
        STORE_SP x13, 13
        STORE_SP x14, 14
        STORE_SP x15, 15
        STORE_SP x16, 16
        STORE_SP x17, 17
        STORE_SP x18, 18
        STORE_SP x19, 19
        STORE_SP x20, 20
        STORE_SP x21, 21
        STORE_SP x22, 22
        STORE_SP x23, 23
        STORE_SP x24, 24
        STORE_SP x25, 25
        STORE_SP x26, 26
        STORE_SP x27, 27
        STORE_SP x28, 28
        STORE_SP x29, 29
        STORE_SP x30, 30
        STORE_SP x31, 31

        # 读取 sp; 按照规定，进入内核态后 sscratch 应为 0
        csrrw s0, sscratch, x0
        # 保存 sp
        STORE_SP s0, 2

        # 因为中断处理中我们可能回根据中断来源的类型决定在中断处理完成后是返回到 sepc 还是 sepc + 4
        csrr s0, sepc
        STORE_SP s0, 32

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
        # 和保存时相反的顺序，避免覆盖，这里我们先还原 sepc，因为这需要用到通用寄存器
        LOAD_SP s0, 32
        csrw sepc, s0

        LOAD_SP x0, 0
        LOAD_SP x1, 1
        LOAD_SP x3, 3
        LOAD_SP x4, 4
        LOAD_SP x5, 5
        LOAD_SP x6, 6
        LOAD_SP x7, 7
        LOAD_SP x8, 8
        LOAD_SP x9, 9
        LOAD_SP x10, 10
        LOAD_SP x11, 11
        LOAD_SP x12, 12
        LOAD_SP x13, 13
        LOAD_SP x14, 14
        LOAD_SP x15, 15
        LOAD_SP x16, 16
        LOAD_SP x17, 17
        LOAD_SP x18, 18
        LOAD_SP x19, 19
        LOAD_SP x20, 20
        LOAD_SP x21, 21
        LOAD_SP x22, 22
        LOAD_SP x23, 23
        LOAD_SP x24, 24
        LOAD_SP x25, 25
        LOAD_SP x26, 26
        LOAD_SP x27, 27
        LOAD_SP x28, 28
        LOAD_SP x29, 29
        LOAD_SP x30, 30
        LOAD_SP x31, 31
        # restore sp last
        LOAD_SP x2, 2
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




