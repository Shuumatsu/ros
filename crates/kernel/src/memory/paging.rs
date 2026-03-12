use alloc::boxed::Box;
use paging::sv39::{READ, READ_EXECUTE, READ_WRITE};
use riscv::{asm::sfence_vma_all, register::satp};
use spin::{Lazy, Mutex};

use crate::memory::layout::{
    bss_end, bss_start, data_end, data_start, heap_start, kernel_stack_end, kernel_stack_start,
    memory_end, rodata_end, rodata_start, text_end, text_start,
};
use crate::platform::{CLINT_BASE, CLINT_SIZE, PLIC_BASE, PLIC_END, UART0_BASE};

pub use paging::sv39::{PAGE_SIZE, PhysicalAddr, Table, VirtualAddr};

/// Verify identity mapping at the given address.
fn verify_id_map(root: *const Table, addr: usize) {
    let expected = Some(PhysicalAddr::new(addr));
    let mapped = Table::translate(root, VirtualAddr::new(addr));
    assert!(mapped == expected, "expect {:?}, but get {:?}", expected, mapped);
}

pub static ROOT_TABLE: Lazy<Mutex<Box<Table>>> = Lazy::new(|| unsafe {
    let ret = Mutex::new(Box::new(Table::new()));

    {
        let root = ret.lock().as_mut() as *mut _;

        // UART
        Table::id_map_range(root, UART0_BASE, UART0_BASE + PAGE_SIZE, READ_WRITE);
        verify_id_map(root, UART0_BASE);

        // CLINT
        Table::id_map_range(root, CLINT_BASE, CLINT_BASE + CLINT_SIZE, READ_WRITE);
        verify_id_map(root, CLINT_BASE);

        // PLIC
        Table::id_map_range(root, PLIC_BASE, PLIC_END, READ_WRITE);
        verify_id_map(root, PLIC_BASE);

        // text section
        Table::id_map_range(root, text_start(), text_end(), READ_EXECUTE);
        verify_id_map(root, text_start());

        // rodata section
        Table::id_map_range(root, rodata_start(), rodata_end(), READ);
        verify_id_map(root, rodata_start());

        // data section
        Table::id_map_range(root, data_start(), data_end(), READ_WRITE);
        verify_id_map(root, data_start());

        // bss section
        Table::id_map_range(root, bss_start(), bss_end(), READ_WRITE);
        verify_id_map(root, bss_start());

        // kernel stack
        Table::id_map_range(root, kernel_stack_start(), kernel_stack_end() + PAGE_SIZE, READ_WRITE);
        verify_id_map(root, kernel_stack_start());

        // heap
        Table::id_map_range(root, heap_start(), memory_end(), READ_WRITE);
        verify_id_map(root, heap_start());
    }

    ret
});

pub unsafe fn init() {
    let root = ROOT_TABLE.lock();
    let addr = root.as_ref() as *const _ as usize;
    let ppn = PhysicalAddr::new(addr).extract_ppn_all();

    unsafe { satp::set(satp::Mode::Sv39, 0, ppn) };
    sfence_vma_all();
}
