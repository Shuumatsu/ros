//! `hello` — the first user program: print a line, run for a while, then exit.
//!
//! The program is the three calls in [`_start`]. What it takes to make them is [`abi::call`]'s, and
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
    spin();
    exit(0)
}

/// Iterations of [`spin`]: enough to cross several timer interrupts on an emulated machine, and
/// short enough that the program still finishes while a boot log is being read.
const SPINS: usize = 5_000_000;

/// Run for long enough to be interrupted, and no longer.
///
/// A hart takes a timer interrupt every ten milliseconds, so a program that exits as soon as it
/// starts never shows that one can be taken in user mode and returned from. The bound is what keeps
/// this a demonstration rather than a hang.
fn spin() {
    let mut sum = 0usize;
    for step in 0..SPINS {
        sum = sum.wrapping_add(step);
    }
    core::hint::black_box(sum);
}

/// Nowhere to report to but the kernel, so a panic is an exit with a status that says so.
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! { exit(101) }
