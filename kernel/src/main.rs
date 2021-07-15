#![no_std]
#![allow(unused)]
#![no_main]
#![feature(const_panic, panic_info_message)]
#![feature(const_size_of_val)]
#![feature(core_intrinsics)]
#![feature(global_asm, llvm_asm, asm)]
#![feature(alloc_error_handler)]
#![feature(alloc_prelude)]
extern crate alloc;

use core::intrinsics::volatile_load;
use core::sync::atomic::{AtomicBool, Ordering};

#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate static_assertions;
#[macro_use]
extern crate bitflags;

use log::{info, warn, LevelFilter};
use spin::Mutex;

#[macro_use]
mod console;
#[macro_use]
mod utils;
mod batch;
mod collections;
mod config;
mod cpu;
mod lang_items;
mod logger;
mod memory;
mod sbi;
mod syscall;
mod trap;

use crate::cpu::CPU;
use crate::memory::layout::{
    BSS_END, BSS_START, DATA_END, DATA_START, KERNEL_STACK_END, KERNEL_STACK_START, RODATA_END,
    RODATA_START, TEXT_END, TEXT_START,
};

global_asm!(include_str!("entry.S"));

static HAS_STARTED: AtomicBool = AtomicBool::new(false);

#[no_mangle]
fn rust_main(hart_id: usize) -> ! {
    println!("main hart initializing");

    logger::init();

    info!("=== memory layout ===");
    info!("text_start: {:#x}, text_end: {:#x}", *TEXT_START, *TEXT_END);
    info!(
        "rodata_start: {:#x}, rodata_end: {:#x}",
        *RODATA_START, *RODATA_END
    );
    info!("data_start: {:#x}, data_end: {:#x}", *DATA_START, *DATA_END);
    info!("bss_start: {:#x}, bss_end: {:#x}", *BSS_START, *BSS_END);

    trap::init();

    HAS_STARTED.store(true, Ordering::SeqCst);

    let cpu = CPU { hart_id };
    println!("main hart {} started", cpu.hart_id);

    loop {}
}

#[no_mangle]
fn rust_main_ap(hart_id: usize) -> ! {
    while !HAS_STARTED.load(Ordering::SeqCst) {}

    let cpu = CPU { hart_id };
    println!("hart {} started", cpu.hart_id);

    loop {}
}
