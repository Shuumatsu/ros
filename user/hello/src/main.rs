//! `hello` — the first user program: print a line, then exit.
//!
//! The program is the two calls in [`_start`]. What it takes to make them is [`abi::call`]'s, and
//! what happens before `_start` is the kernel's loader's.

#![no_std]
#![no_main]

use core::panic::PanicInfo;

use abi::call::{exit, write};
use abi::syscall::STDOUT;

/// The first byte of the image, and the first instruction this hart runs in user mode.
///
/// No arguments, because there is no `argv` to pass yet. No return, because `ra` holds nothing on
/// entry: the only way out is [`exit`].
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
extern "C" fn _start() -> ! {
    write(STDOUT, b"hello, world\n");
    exit(0)
}

/// Nowhere to report to but the kernel, so a panic is an exit with a status that says so.
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! { exit(101) }
