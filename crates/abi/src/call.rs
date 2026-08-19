//! The calling side: `ecall`, and one wrapper per number.
//!
//! Only a user program compiles this — the kernel is on the receiving end and reads
//! [`super::syscall`] to dispatch.

use core::arch::asm;

use super::syscall;

/// The one way out of user mode: arguments in `a0`–`a2`, number in `a7`, result in `a0`.
///
/// No `options`, because the conservative default is the truthful one: a system call reads and
/// writes the caller's memory, and some of these hand the kernel a pointer into it.
///
/// # Safety
///
/// `number` must be one the kernel implements, and the arguments must be what that call expects —
/// a pointer argument is dereferenced by the kernel on this process's behalf.
unsafe fn syscall(number: usize, a0: usize, a1: usize, a2: usize) -> isize {
    let result: isize;
    // SAFETY: forwarded from this function's contract.
    unsafe {
        asm!(
            "ecall",
            in("a7") number,
            inlateout("a0") a0 => result,
            in("a1") a1,
            in("a2") a2,
        );
    }
    result
}

/// Write `bytes` to `fd`, answering with the count written or a negative error.
pub fn write(fd: usize, bytes: &[u8]) -> isize {
    // SAFETY: `bytes` is a live slice, so its pointer and length describe memory this process owns.
    unsafe { syscall(syscall::WRITE, fd, bytes.as_ptr() as usize, bytes.len()) }
}

/// End this process with `status`.
pub fn exit(status: usize) -> ! {
    // SAFETY: no pointer is involved, and the kernel does not return from this one.
    unsafe { syscall(syscall::EXIT, status, 0, 0) };
    unreachable!("the kernel returned from exit")
}
