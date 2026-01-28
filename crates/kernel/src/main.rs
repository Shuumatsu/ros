#![no_std]
#![no_main]
#![feature(lang_items)]
#![feature(alloc_error_handler)]
#![feature(naked_functions)]

extern crate alloc;

use core::arch::global_asm;
#[macro_use]
extern crate static_assertions;

mod arch;
mod collections;
mod cpu;
mod drivers;
mod isa;
mod lang_items;
#[macro_use]
mod console;
mod memory;
mod platform;
mod proc;
mod sbi;
mod start;
mod trap;
mod utils;

global_asm!(include_str!("boot.S"));

// the -> ! means that this function won't return
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    if let Some(p) = info.location() {
        kprintln!("Aborting: file {}:{}: \n\t{}", p.file(), p.line(), info.message());
    } else {
        kprintln!("Aborting: no information available.");
    }
    abort();
}

// https://internals.rust-lang.org/t/why-rust-has-name-mangling/12503
// turns off Rust's name mangling so the symbol is exactly eh_personality
#[unsafe(no_mangle)]
extern "C" fn abort() -> ! {
    kprintln!("[cpu: {}] enter extern \"C\" fn abort()", arch::riscv64::hart_id());
    loop {
        riscv::asm::wfi();
    }
}

// eh_personality language item marks a function that is used for implementing stack unwinding
// By default, Rust uses unwinding to run the destructors of all live stack variables in case of a panic.
#[lang = "eh_personality"]
extern "C" fn eh_personality() {}
