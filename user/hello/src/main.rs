//! `hello` — the first user program: print a line, then exit.
//!
//! The program is the two calls in [`_start`]. What it takes to make them is [`abi::call`]'s, and
//! what happens before `_start` is the kernel's loader's.

#![no_std]
#![no_main]

use core::panic::PanicInfo;

use abi::call::{exit, write};
use abi::syscall::STDOUT;

/// The entry point, named in `hello.ld` and reported to the kernel's loader as `e_entry`.
///
/// No arguments, because there is no `argv` to pass yet. No return, because `ra` holds nothing on
/// entry: the only way out is [`exit`].
#[unsafe(no_mangle)]
extern "C" fn _start() -> ! {
    write(STDOUT, b"hello, world\n");
    exit(0)
}

/// Nowhere to report to but the kernel, so a panic is an exit with a status that says so.
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! { exit(101) }
