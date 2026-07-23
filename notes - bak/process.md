for each process:
- kernel stack
- user stack
- pagetable



kernel stack
for running syscalls handlers
- on syscall 
- jump trap addr page


1. 用户线程的 user mode ↔ 同一线程的 kernel mode
   原因：syscall、exception、return-to-user

2. 任意执行状态 → interrupt context → 原执行状态
   原因：IRQ、IPI、timer、NMI 等

3. 任意 task A/kernel context → scheduler → task B/kernel context
   A、B 可以是用户线程、内核线程或 idle

4. task B/kernel context → B/user mode
   仅当 B 是用户线程并准备返回用户空间

真正的 task switch 永远发生在内核态，并且本质上是从 A 的 kernel stack 切到 B 的 kernel stack；从用户视角看到的 A/user → B/user 只是这个完整过程的首尾。
```
User mode
    either:
        execute ecall
        OR an interrupt becomes pending

    Hardware:
        sepc =
        syscall: address of ecall
        interrupt: address where user execution should resume
        scause =
        syscall: "environment call from U-mode"
        interrupt: interrupt flag + interrupt type
        sstatus records previous privilege/interrupt state
        privilege becomes S-mode
        interrupts are disabled
        pc = stvec

    stvec → uservec (trampoline)
    CPU does NOT automatically save general registers
    uservec:
        obtain trapframe address via sscratch
        save user registers into the process's trapframe
        load kernel stack pointer and kernel trap-handler address
        switch satp to the kernel page table
        sfence.vma
        jump to usertrap

    usertrap:
    set stvec to kernelvec for traps occurring in the kernel
    save sepc into the process's trapframe
    inspect scause

    if syscall:
        advance saved user pc by 4, past ecall
        read syscall number from a7
        read arguments from a0–a5
        call syscall handler
        place return value in a0

    else if interrupt:
        do NOT advance saved user pc

        if timer interrupt:
        handle timer bookkeeping
        possibly yield and schedule another process

        else if external-device interrupt:
        identify the device
        run its interrupt handler
        acknowledge/complete the interrupt

        else if software interrupt:
        handle the software interrupt

    else:
        handle another exception, such as a page fault
        or terminate the process if it cannot be handled

    usertrapret:
    disable interrupts during return setup
    prepare sstatus for return to U-mode
    load sepc from the saved user pc
    set stvec back to uservec
    jump to userret in the trampoline

    userret:
    switch satp back to the user page table
    sfence.vma
    restore registers from the trapframe
    sret
        privilege returns to U-mode
        pc = sepc
```