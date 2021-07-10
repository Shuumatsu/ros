#![no_std]
#![no_main]
#![feature(llvm_asm)]
#![feature(core_intrinsics)]
#![feature(panic_info_message)]
#![feature(global_asm)]

use core::intrinsics::volatile_load;
use core::sync::atomic::{AtomicBool, Ordering};

use log::{info, warn, LevelFilter};
use spin::Mutex;

#[macro_use]
extern crate lazy_static;
#[macro_use]
mod console;
mod lang_items;
mod logger;
mod memory;
mod sbi;

use crate::logger::ColorLogger;
use crate::memory::layout::{
    BSS_END, BSS_START, DATA_END, DATA_START, KERNEL_STACK_END, KERNEL_STACK_START, RODATA_END,
    RODATA_START, TEXT_END, TEXT_START,
};
use crate::sbi::shutdown;

global_asm!(include_str!("entry.asm"));

static HAS_STARTED: AtomicBool = AtomicBool::new(false);

#[no_mangle]
extern "C" fn rust_entry(hart_id: usize) -> ! {
    if hart_id == 0 {
        rust_main(hart_id)
    } else {
        rust_main_ap(hart_id)
    }
}

#[no_mangle]
fn rust_main(hart_id: usize) -> ! {
    println!(
        "main hart {} started, {:x}",
        hart_id, &hart_id as *const _ as usize
    );

    logger::init();

    info!(".text [{:#x}, {:#x})", *TEXT_START, *TEXT_END);
    info!(".rodata [{:#x}, {:#x})", *RODATA_START, *RODATA_END);
    info!(".data [{:#x}, {:#x})", *DATA_START, *DATA_END);
    info!(".bss [{:#x}, {:#x})", *BSS_START, *BSS_END);

    HAS_STARTED.store(true, Ordering::SeqCst);

    loop {}
}

#[no_mangle]
fn rust_main_ap(hart_id: usize) -> ! {
    while !HAS_STARTED.load(Ordering::SeqCst) {}

    println!(
        "hart {} started, {:x}",
        hart_id, &hart_id as *const _ as usize
    );

    loop {}
}
