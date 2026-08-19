//! `hello` — the first user program: print a line, then exit.
//!
//! Freestanding for the kernel's reasons and one more: there is no libc to link against, and the
//! only interface out of user mode is `ecall`. So the whole program is the two syscalls below,
//! and what runs before `_start` is the loader.

#![no_std]
#![no_main]

use core::arch::asm;
use core::panic::PanicInfo;

/// Syscall numbers, as Linux numbers them on riscv64.
///
/// Borrowed rather than invented so that a user program built against a real libc would agree
/// with this kernel about what it is asking for.
const WRITE: usize = 64;
const EXIT: usize = 93;

/// The file descriptor the kernel writes to its console.
const STDOUT: usize = 1;

/// The one way out of user mode: arguments in `a0`–`a2`, number in `a7`, result in `a0`.
///
/// No `options`, because the conservative default is the truthful one — a syscall reads and
/// writes the caller's memory, and this one hands the kernel a pointer into it.
unsafe fn syscall(number: usize, a0: usize, a1: usize, a2: usize) -> isize {
    let result: isize;
    // SAFETY: the ABI above is what the kernel's `ecall` handler implements.
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

fn write(fd: usize, bytes: &[u8]) -> isize {
    // SAFETY: `bytes` is a live slice, so the pointer and length describe memory this program owns.
    unsafe { syscall(WRITE, fd, bytes.as_ptr() as usize, bytes.len()) }
}

fn exit(code: usize) -> ! {
    // SAFETY: `exit` does not return, so there is nothing for the kernel to corrupt.
    unsafe { syscall(EXIT, code, 0, 0) };
    unreachable!("the kernel returned from exit")
}

/// The first byte of the image, and the first instruction the hart runs in user mode.
///
/// No arguments: there is no `argv` to pass yet. No return: `ra` holds nothing on entry, so the
/// only way out is [`exit`].
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
extern "C" fn _start() -> ! {
    write(STDOUT, b"hello, world\n");
    exit(0)
}

/// Nowhere to report to but the kernel, so a panic is an exit with a status that says so.
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! { exit(101) }
