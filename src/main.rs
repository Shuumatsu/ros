#![no_std]
#![no_main]
#![feature(panic_info_message, global_asm, llvm_asm, asm, lang_items)]

mod arch;
mod assembly;
mod echo;
mod macros;
mod mem;
mod start;
mod uart;
mod utils;

// the -> ! means that this function won't return
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    print!("Aborting: ");
    if let Some(p) = info.location() {
        println!(
            "line {}, file {}: {}",
            p.line(),
            p.file(),
            info.message().unwrap()
        );
    } else {
        println!("no information available.");
    }
    abort();
}

// https://internals.rust-lang.org/t/why-rust-has-name-mangling/12503
// turns off Rust's name mangling so the symbol is exactly eh_personality
#[no_mangle]
extern "C" fn abort() -> ! {
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
