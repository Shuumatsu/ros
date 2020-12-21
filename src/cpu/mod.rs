use crate::memory::layout::*;
use crate::{kprint, kprintln};

pub fn print_info() {
    kprintln!("memory layout: ");
    kprintln!("    memory_start: {:#x}, memory_end: {:#x}", memory_start(), memory_end());
    kprintln!("    text_start: {:#x}, text_end: {:#x}", text_start(), text_end());
    kprintln!("    rodata_start: {:#x}, rodata_end: {:#x}", rodata_start(), rodata_end());
    kprintln!("    data_start: {:#x}, data_end: {:#x}", data_start(), data_end());
    kprintln!("    bss_start: {:#x}, bss_end: {:#x}", bss_start(), bss_end());
    kprintln!(
        "    kernel_stack_start: {:#x}, kernel_stack_end: {:#x}",
        kernel_stack_start(),
        kernel_stack_end()
    );
    kprintln!("    heap_start: {:#x}, heap_end: {:#x}", heap_start(), heap_start() + heap_size());
}
