#![no_std]
#![feature(llvm_asm, asm)]
#![feature(linkage)]
#![feature(panic_info_message)]

#[macro_use]
extern crate lazy_static;

#[macro_use]
pub mod console;
pub mod lang_items;
pub mod memory;
pub mod prelude;
mod syscall;

use memory::layout::{BSS_END, BSS_START};

pub use syscall::*;

pub const STDOUT: usize = 1;

#[no_mangle]
#[link_section = ".text.entry"]
pub extern "C" fn _start() -> ! {
    (*BSS_START..*BSS_END).for_each(|addr| unsafe {
        (addr as *mut u8).write_volatile(0);
    });

    exit(main());

    panic!("unreachable after sys_exit!");
}

#[linkage = "weak"]
#[no_mangle]
fn main() -> i32 {
    panic!("Cannot find main!");
}
