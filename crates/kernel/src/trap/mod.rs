// mod context;
mod exceptions;
mod interrupts;
mod trapframe;

use core::arch::global_asm;

use riscv::interrupt::supervisor::{Exception, Interrupt};
use riscv::interrupt::Trap;
use riscv::register::*;

// use context::*;
pub use trapframe::*;

global_asm!(include_str!("trampoline.S"));

#[allow(improper_ctypes)]
unsafe extern "C" {
    safe fn trap_entry();
    // fn run_user(regs: &mut UserContext);
}

// sscratch: 一个字的临时存储空间，一般用来辅助中断处理
// sstatus: 系统状态寄存器
// stvec: 中断跳转地址
// scause: 中断或异常的原因
// sepc: 发生中断时的位置 / PC

// RISC-V 将异常分为两类。
//     - 一类是同步异常，这类异常在指令执行期间产生，如访问了无效的存储器地址或执行了具有无效操作码的指令时。
//       在 M 模式运行期间可能发生的同步例外有五种：
//         - 访问错误异常 当物理内存的地址不支持访问类型时发生（例如尝试写入 ROM）。
//         - 断点异常 在执行 ebreak 指令，或者地址或数据与调试触发器匹配时发生。
//         - 环境调用异常 在执行 ecall 指令时发生。
//         - 非法指令异常 在译码阶段发现无效操作码时发生。
//         - 非对齐地址异常 在有效地址不能被访问大小整除时发生。
//     - 另一类是中断，它是与指令流异步的外部事件，比如鼠标的单击。
//       有三种标准的中断源：软件、时钟和外部来源。
//         - 软件：通过像向内存映射寄存器种存数并通常用一个 hart 来中断另一个 hart
//         - 时钟：当实时计数器 mtime 大于 hart 的时间比较器（一个名为 mtimecmp 的内存映射寄存器）时触发时钟中断
//         - 外部来源：由平台级中断控制器引发（大部分外部设备连接到这个中断控制器）

// RISC V 的异常 are precise：所有异常前的指令已完全执行 & 所有异常后的指令还未开始执行

// 当一个 hart 发生异常时，硬件自动做以下处理
//     1. 异常指令的 PC vei存在 mepc 中；PC 被设置为 mtvec
//     2. 根据异常来源设置 mcause 并设置 mtval
//     3. mstatus.mpie = mstatus.mie; mstatus.mie = 0
//     4. 将异常前的权限模式保存在 mstatus.mpp 中，并切换到 machine mode

// 当我们的程序遇上中断或异常时，cpu 会跳转到一个指定的地址进行中断处理。
// 在 RISCV 中，这个地址由 stvec 控制寄存器保存。init 将其设置为 trap_handler 的地址
pub unsafe fn init() {
    unsafe {
        interrupts::init();
        exceptions::init();

        // stvec 中包含了向量基址（BASE） 和向量模式（MODE）
        // 向量基址（BASE） 必须按照 4 字节对齐。
        let addr = trap_entry as usize;
        // 直接模式（Driect） MODE = 0 ，触发任何中断异常 时都把 PC 设置为 BASE
        // 向量模式（Vectored） MODE = 1 ，对第 i 种中断 ，跳转到 BASE + i * 4；对所有异常，仍跳转到 BASE
        // 我们采用第一种模式，先进入统一的处理函数，之后再根据中断 / 异常种类进行不同处理。
        println!("[interrupts::init] set stec register: trap_entry {:#x}, mode Direct", addr);
        stvec::write(stvec::Stvec::new(addr, stvec::TrapMode::Direct));

        // 当中断发生时，cpu 跳转到中断处理函数。sscratch 存储了函数将要用到的 sp
        // 我们用 sscratch 是否为 0 来区分中断是来自内核还是来自用户
        // 如果来自内核，则继续使用操作系统的栈即可
        // 如果来自用户，则需要切换到为进程分配的内核栈；此时我们交换 sscratch 与 sp 以保存用户的 sp
        sscratch::write(0);
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn trap_handler(tf: &mut TrapFrame) {
    kprintln!("[trap_handler] enter trap_handler");

    let scause = scause::read();
    let epc = sepc::read();
    kprintln!("[trap_handler] scause code: {}, sepc: {:#x}", scause.code(), epc);

    let cause: Trap<Interrupt, Exception> = scause.cause().try_into().expect("unknown trap cause");
    match cause {
        Trap::Exception(e) => exceptions::handler(e, tf),
        Trap::Interrupt(intr) => unsafe { interrupts::handler(intr, tf) },
    }
}

/// Saved kernel context for returning from user-space on SYS_EXIT.
/// Layout: ra, sp, s0-s11 (14 registers × 8 bytes = 112 bytes)
#[repr(C, align(8))]
struct KernelContext([usize; 14]);

static mut KERNEL_CTX: KernelContext = KernelContext([0; 14]);

/// Drop to U-mode and execute the program at `entry` with the given user stack pointer.
/// Returns when the user program issues a SYS_EXIT ecall.
///
/// Before `sret`, we save the kernel's callee-saved registers (ra, sp, s0-s11)
/// into KERNEL_CTX. The SYS_EXIT handler calls `return_to_kernel()` which
/// restores them and returns here.
#[unsafe(naked)]
pub unsafe extern "C" fn run_user_program(entry: usize, user_sp: usize) {
    // a0 = entry, a1 = user_sp
    core::arch::naked_asm!(
        // Save kernel callee-saved registers into KERNEL_CTX
        "la t0, {ctx}",
        "sd ra,  0*8(t0)",
        "sd sp,  1*8(t0)",
        "sd s0,  2*8(t0)",
        "sd s1,  3*8(t0)",
        "sd s2,  4*8(t0)",
        "sd s3,  5*8(t0)",
        "sd s4,  6*8(t0)",
        "sd s5,  7*8(t0)",
        "sd s6,  8*8(t0)",
        "sd s7,  9*8(t0)",
        "sd s8, 10*8(t0)",
        "sd s9, 11*8(t0)",
        "sd s10,12*8(t0)",
        "sd s11,13*8(t0)",

        // Set sstatus.SPP = User (clear bit 8)
        "csrr t0, sstatus",
        "li t1, ~(1 << 8)",
        "and t0, t0, t1",
        // Also clear SIE (bit 1) — no interrupts during user execution
        "li t1, ~(1 << 1)",
        "and t0, t0, t1",
        "csrw sstatus, t0",

        // sepc = entry (a0)
        "csrw sepc, a0",

        // sscratch = kernel sp (trampoline uses sscratch != 0 to detect U-mode trap)
        "csrw sscratch, sp",

        // Switch to user stack and sret to U-mode
        "mv sp, a1",
        "sret",
        ctx = sym KERNEL_CTX,
    );
}

/// Called from the SYS_EXIT ecall handler to restore kernel context
/// and return to the caller of `run_user_program`.
pub unsafe fn return_to_kernel() -> ! {
    unsafe {
        core::arch::asm!(
            "la t0, {ctx}",
            "ld ra,  0*8(t0)",
            "ld sp,  1*8(t0)",
            "ld s0,  2*8(t0)",
            "ld s1,  3*8(t0)",
            "ld s2,  4*8(t0)",
            "ld s3,  5*8(t0)",
            "ld s4,  6*8(t0)",
            "ld s5,  7*8(t0)",
            "ld s6,  8*8(t0)",
            "ld s7,  9*8(t0)",
            "ld s8, 10*8(t0)",
            "ld s9, 11*8(t0)",
            "ld s10,12*8(t0)",
            "ld s11,13*8(t0)",
            "ret",
            ctx = sym KERNEL_CTX,
            options(noreturn),
        );
    }
}

pub fn wait_for_interrupt() {
    unsafe {
        let prev_sie = sstatus::read().sie();

        sstatus::set_sie();
        riscv::asm::wfi();

        // restore prev sie
        if !prev_sie {
            sstatus::clear_sie();
        }
    }
}
