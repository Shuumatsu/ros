#![no_std]
#![feature(llvm_asm, asm)]
#![feature(linkage)]
#![feature(panic_info_message)]

#[macro_use]
extern crate lazy_static;

#[macro_use]
pub mod console;
pub mod lang_items;
pub mod prelude;
mod syscall;

pub use syscall::*;

pub const STDOUT: usize = 1;

#[no_mangle]
#[link_section = ".text.entry"]
pub extern "C" fn _start() -> ! {
    exit(main());

    panic!("unreachable after sys_exit!");
}

#[linkage = "weak"]
#[no_mangle]
fn main() -> i32 {
    panic!("Cannot find main!");
}
