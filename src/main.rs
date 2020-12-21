#![no_std]
#![no_main]
#![feature(const_panic, panic_info_message)]
#![feature(lang_items)]
#![feature(global_asm, llvm_asm, asm)]
#![feature(alloc_error_handler)]
#![feature(naked_functions)]
// https://doc.rust-lang.org/alloc/prelude/index.html
#![feature(alloc_prelude)]
extern crate alloc;
use alloc::prelude::v1::*;
#[macro_use]
extern crate static_assertions;

mod allocator;
mod arch;
mod collections;
mod context;
mod isa;
mod cpu;
mod echo;
mod interrupt;
mod log;
mod memory;
mod start;
mod uart;
mod utils;

global_asm!(include_str!("boot.S"));

// the -> ! means that this function won't return
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    if let Some(p) = info.location() {
        kprintln!("Aborting: file {}:{}: \n\t{}", p.file(), p.line(), info.message().unwrap());
    } else {
        kprintln!("Aborting: no information available.");
    }
    abort();
}

// https://internals.rust-lang.org/t/why-rust-has-name-mangling/12503
// turns off Rust's name mangling so the symbol is exactly eh_personality
#[no_mangle]
extern "C" fn abort() -> ! {
    println!("[cpu: {}] enter extern \"C\" fn abort()", arch::riscv64::hart_id());
    loop {
        unsafe {
            riscv::asm::wfi();
        }
    }
}

// eh_personality language item marks a function that is used for implementing stack unwinding
// By default, Rust uses unwinding to run the destructors of all live stack variables in case of a panic.
#[lang = "eh_personality"]
extern "C" fn eh_personality() {}
