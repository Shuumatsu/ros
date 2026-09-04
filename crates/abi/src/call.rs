//! RISC-V user-side system-call wrappers.

use core::arch::asm;

use super::syscall;

/// Issues an `ecall` with its number in `a7`, arguments in `a0`–`a2`, and result in `a0`.
///
/// No assembly options are declared because a call may access caller memory.
///
/// # Safety
///
/// `number` and its arguments must satisfy the selected system call's ABI.
unsafe fn syscall(number: usize, a0: usize, a1: usize, a2: usize) -> isize {
    let result: isize;
    // SAFETY: The caller satisfies the selected system call's ABI.
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

/// Writes `bytes` to `fd`, returning a count or negated `errno`.
pub fn write(fd: usize, bytes: &[u8]) -> isize {
    // SAFETY: The slice describes live caller-owned memory.
    unsafe { syscall(syscall::WRITE, fd, bytes.as_ptr() as usize, bytes.len()) }
}

/// Terminates the process with `status`.
pub fn exit(status: usize) -> ! {
    // SAFETY: `exit` takes no pointer and does not return.
    unsafe { syscall(syscall::EXIT, status, 0, 0) };
    unreachable!("the kernel returned from exit")
}
