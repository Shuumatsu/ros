use crate::memory::layout::*;
use crate::{print, println};

pub fn print_info() {
    println!("memory layout: ");
    println!("    memory_start: {:#x}, memory_end: {:#x}", memory_start(), memory_end());
    println!("    text_start: {:#x}, text_end: {:#x}", text_start(), text_end());
    println!("    rodata_start: {:#x}, rodata_end: {:#x}", rodata_start(), rodata_end());
    println!("    data_start: {:#x}, data_end: {:#x}", data_start(), data_end());
    println!("    bss_start: {:#x}, bss_end: {:#x}", bss_start(), bss_end());
    println!(
        "    kernel_stack_start: {:#x}, kernel_stack_end: {:#x}",
        kernel_stack_start(),
        kernel_stack_end()
    );
    println!("    heap_start: {:#x}, heap_end: {:#x}", heap_start(), heap_start() + heap_size());
}
