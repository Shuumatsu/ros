#![no_std]
#![no_main]
#![feature(alloc_error_handler)]
#![feature(abi_custom)]

extern crate alloc;

mod arch;
mod cpu;
#[macro_use]
mod console;
mod device_tree;
mod memory;
mod start;
mod utils;

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
/// No `eh_personality` is defined anywhere in this crate, and none is needed: both
/// profiles set `panic = "abort"` and the target spec for
/// `riscv64imac-unknown-none-elf` says `"panic-strategy": "abort"`, so nothing
/// unwinds.
#[unsafe(no_mangle)]
extern "C" fn abort() -> ! {
    // No hart id in the message: `_emergency_print` already prefixes every line.
    emergency_println!("enter abort()");
    arch::riscv64::wait_forever()
}
