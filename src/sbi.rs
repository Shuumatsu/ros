#![allow(unused)]

const SBI_SET_TIMER: usize = 0;
const SBI_CONSOLE_PUTCHAR: usize = 1;
const SBI_CONSOLE_GETCHAR: usize = 2;
const SBI_CLEAR_IPI: usize = 3;
const SBI_SEND_IPI: usize = 4;
const SBI_REMOTE_FENCE_I: usize = 5;
const SBI_REMOTE_SFENCE_VMA: usize = 6;
const SBI_REMOTE_SFENCE_VMA_ASID: usize = 7;
const SBI_SHUTDOWN: usize = 8;
// SBI_SET_TIMER 这些常量
// 我的理解是这些并不是属于我们 kernel 的东西 而是定义在 rustsbi 中的
// 因为我们的 os 直接用的 rustsbi 的二进制文件 所以这里直接是直接从rustsbi源码中复制过来的
// 这里是和类似 use rustsbit::SBI_SET_TIMER 这样的句子等价的

#[inline(always)]
fn sbi_call(which: usize, arg0: usize, arg1: usize, arg2: usize) -> usize {
    let mut ret;
    unsafe {
        llvm_asm!("ecall"
            : "={x10}" (ret)
            : "{x10}" (arg0), "{x11}" (arg1), "{x12}" (arg2), "{x17}" (which)
            : "memory"
            : "volatile"
        );
    }
    ret
}

#[inline(always)]
pub fn console_putchar(c: usize) {
    sbi_call(SBI_CONSOLE_PUTCHAR, c, 0, 0);
}

#[inline(always)]
pub fn console_getchar() -> usize {
    sbi_call(SBI_CONSOLE_GETCHAR, 0, 0, 0)
}

#[inline(always)]
pub fn shutdown() -> ! {
    sbi_call(SBI_SHUTDOWN, 0, 0, 0);
    panic!("It should shutdown!");
}

#[inline(always)]
pub fn hart_id() -> usize {
    let mut hart_id: usize = 0;

    unsafe {
        llvm_asm!("mv $0, tp" : "=r"(hart_id) ::: "volatile");
    }
    hart_id
}
