// mod context;
mod trapframe;

use riscv::register::scause::{Exception as E, Scause, Trap};
use riscv::register::*;

// use context::*;
use trapframe::*;

use crate::{kprint, kprintln};

pub const CLINT_BASE_ADDR: usize = 0x0200_0000;

global_asm!(include_str!("trap.S"));

#[allow(improper_ctypes)]
extern "C" {
    fn trap_entry();
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
    kprintln!("delegate all interrupts and exceptions to supervisor mode");
    asm!("li t0, 0xffff");
    asm!("csrw medeleg, t0");
    asm!("li t0, 0xffff");
    asm!("csrw mideleg, t0");

    kprintln!("enable all supervisor interrupts");
    asm!("li t0, 0xffff");
    asm!("csrw sie, t0");

    // stvec 中包含了向量基址（BASE） 和向量模式（MODE）
    // 向量基址（BASE） 必须按照 4 字节对齐。
    let addr = trap_entry as usize;
    // 直接模式（Driect） MODE = 0 ，触发任何中断异常 时都把 PC 设置为 BASE
    // 向量模式（Vectored） MODE = 1 ，对第 i 种中断 ，跳转到 BASE + i * 4；对所有异常，仍跳转到 BASE
    // 我们采用第一种模式，先进入统一的处理函数，之后再根据中断 / 异常种类进行不同处理。
    let mode = stvec::TrapMode::Direct;
    kprintln!("[interrupts::init] set stec register: trap_entry {:#x}, mode {:?}", addr, mode);
    stvec::write(addr, mode);

    // 当中断发生时，cpu 跳转到中断处理函数。sscratch 存储了函数将要用到的 sp
    // 我们用 sscratch 是否为 0 来区分中断是来自内核还是来自用户
    // 如果来自内核，则继续使用操作系统的栈即可
    // 如果来自用户，则需要切换到为进程分配的内核栈；此时我们交换 sscratch 与 sp 以保存用户的 sp
    sscratch::write(0);
}

#[no_mangle]
extern "C" fn trap_handler(tf: &mut TrapFrame) {
    kprintln!("[trap_handler] enter trap_handler");

    let cause = scause::read().cause();
    let epc = sepc::read();
    kprintln!("[trap_handler] scause: {:?}, sepc: {:#x}", cause, epc);
    kprintln!("[trap_handler] trapframe: {:?}", tf);

    tf.increase_sepc();
}

unsafe extern "C" fn user_entry() {
    kprintln!("enter user_entry");

    unimplemented!("user_entry")
}
