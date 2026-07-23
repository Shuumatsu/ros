use crate::memory::layout::*;
use crate::{print, println};

pub fn print_info() {
    println!("kernel image layout: ");
    println!("    load base: {:#x}", memory_start());
    println!("    text_start: {:#x}, text_end: {:#x}", text_start(), text_end());
    println!("    rodata_start: {:#x}, rodata_end: {:#x}", rodata_start(), rodata_end());
    println!("    data_start: {:#x}, data_end: {:#x}", data_start(), data_end());
    println!("    bss_start: {:#x}, bss_end: {:#x}", bss_start(), bss_end());
    println!(
        "    kernel_stack_start: {:#x}, kernel_stack_end: {:#x}",
        kernel_stack_start(),
        kernel_stack_end()
    );
    // Heap end is discovered from the device tree at runtime; see `memory::init`.
    println!("    heap_start: {:#x}", heap_start());
}
