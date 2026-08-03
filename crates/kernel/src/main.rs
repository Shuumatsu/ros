#![no_std]
#![no_main]
#![feature(alloc_error_handler)]

extern crate alloc;

use core::arch::global_asm;

mod arch;
mod cpu;
#[macro_use]
mod console;
mod device_tree;
mod memory;
mod start;
mod utils;

global_asm!(include_str!("boot.S"));

// the -> ! means that this function won't return
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    if let Some(p) = info.location() {
        emergency_println!("Aborting: file {}:{}: \n\t{}", p.file(), p.line(), info.message());
    } else {
        emergency_println!("Aborting: no information available.");
    }
    abort();
}

/// Last stop on the fatal path. `no_mangle` because the symbol must be exactly
/// `abort` — compiler-generated code refers to it by that name.
///
/// There is no `eh_personality` here, and no `#![feature(lang_items)]` to declare
/// one. Both profiles set `panic = "abort"` and the target spec for
/// `riscv64imac-unknown-none-elf` says `"panic-strategy": "abort"`, so nothing
/// unwinds and the personality routine was never reachable — it only cost a
/// nightly feature gate.
#[unsafe(no_mangle)]
extern "C" fn abort() -> ! {
    // No hart id in the message: `_emergency_print` already prefixes every line
    // with one. This used to print its own as `[cpu: N]`, so the most important
    // message in the kernel read `[hart 0] [cpu: 0] ...` — the same fact twice,
    // under two names.
    emergency_println!("enter abort()");
    arch::riscv64::wait_forever()
}
