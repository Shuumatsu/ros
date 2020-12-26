use alloc::boxed::Box;
use lazy_static::lazy_static;
use riscv::{asm::sfence_vma_all, register::satp};
use spin::Mutex;

pub use crate::arch::riscv64::paging::sv39::{
    virt_to_phys, PhysicalAddr, Table, VirtualAddr, PAGE_SIZE,
};
use crate::memory::layout::{
    bss_end, bss_start, data_end, data_start, heap_size, heap_start, kernel_stack_end,
    kernel_stack_start, rodata_end, rodata_start, text_end, text_start, CLINT_BASE_ADDR,
    PLIC_BASE_ADDR, PLIC_END_ADDR, UART_BASE_ADDR,
};
use crate::utils::{align_down, align_up};
use crate::{arch::riscv64::paging::sv39 as paging, memory::layout::memory_end};
use crate::{print, println};

pub unsafe fn map_range<F: Fn(VirtualAddr) -> PhysicalAddr>(
    root: *mut Table,
    mut start: usize,
    mut end: usize,
    f: F,
    bits: usize,
) {
    start = align_up(start, PAGE_SIZE);
    end = align_down(end, PAGE_SIZE);

    println!(
        "[map_range] root: {:?}, start: {:#x}, end: {:#x}, bits: {:#b}",
        root, start, end, bits
    );

    for curr in (start..end).step_by(PAGE_SIZE) {
        let vaddr = VirtualAddr::new(curr);
        let paddr = f(vaddr);
        paging::map(root, vaddr, paddr, bits);
    }

    println!(
        "[map_range completed] root: {:?}, start: {:#x}, end: {:#x}, bits: {:#b}",
        root, start, end, bits
    );
}

pub unsafe fn id_map_range(root: *mut Table, mut start: usize, mut end: usize, bits: usize) {
    map_range(root, start, end, |vaddr| PhysicalAddr::new(vaddr.extract_bits()), bits);
}

lazy_static! {
    pub static ref ROOT_TABLE: Mutex<Box<Table>> = unsafe {
        let ret = Mutex::new(Box::new(Table::new()));

        {
            let root = ret.lock().as_mut() as *mut _;
            println!("[initialize root table] root page table created at, {:?}", root);


            // UART
            println!("[initialize root table] mapping UART...");
            id_map_range(root, UART_BASE_ADDR, UART_BASE_ADDR + PAGE_SIZE, paging::READ_WRITE);
            println!("[initialize root table] mapping UART completed");

            let expected = Some(PhysicalAddr::new(UART_BASE_ADDR));
            let mapped = virt_to_phys(root, VirtualAddr::new(UART_BASE_ADDR));
            assert!(mapped == expected, "expect {:?}, but get {:?}", expected, mapped);


            // CLINT
            println!("[initialize root table] mapping CLINT...");
            id_map_range(root, CLINT_BASE_ADDR, CLINT_BASE_ADDR + PAGE_SIZE, paging::READ_WRITE);
            println!("[initialize root table] mapping CLINT completed");

            let expected = Some(PhysicalAddr::new(CLINT_BASE_ADDR));
            let mapped = virt_to_phys(root, VirtualAddr::new(CLINT_BASE_ADDR));
            assert!(mapped == expected, "expect {:?}, but get {:?}", expected, mapped);


            // PLIC
            println!("[initialize root table] mapping PLIC...");
            id_map_range(root, PLIC_BASE_ADDR, PLIC_END_ADDR, paging::READ_WRITE);
            println!("[initialize root table] mapping PLIC completed");

            let expected = Some(PhysicalAddr::new(PLIC_BASE_ADDR));
            let mapped = virt_to_phys(root, VirtualAddr::new(PLIC_BASE_ADDR));
            assert!(mapped == expected, "expect {:?}, but get {:?}", expected, mapped);


            // map text section
            println!("[initialize root table] mapping text section...");
            id_map_range(root, text_start(), text_end(), paging::READ_EXECUTE);
            println!("[initialize root table] mapping text section completed");

            let expected = Some(PhysicalAddr::new(text_start()));
            let mapped = virt_to_phys(root, VirtualAddr::new(text_start()));
            assert!(mapped == expected, "expect {:?}, but get {:?}", expected, mapped);


            // map rodata section
            println!("[initialize root table] mapping rodata section...");
            id_map_range(root, rodata_start(), rodata_end(), paging::READ);
            println!("[initialize root table] mapping rodata section completed");

            let expected = Some(PhysicalAddr::new(rodata_start()));
            let mapped = virt_to_phys(root, VirtualAddr::new(rodata_start()));
            assert!(mapped == expected, "expect {:?}, but get {:?}", expected, mapped);


            // map data section
            println!("[initialize root table] mapping data section...");
            id_map_range(root, data_start(), data_end(), paging::READ_WRITE);
            println!("[initialize root table] mapping data section completed");

            let expected = Some(PhysicalAddr::new(data_start()));
            let mapped = virt_to_phys(root, VirtualAddr::new(data_start()));
            assert!(mapped == expected, "expect {:?}, but get {:?}", expected, mapped);


            // map bss section
            println!("[initialize root table] mapping bss section...");
            id_map_range(root, bss_start(), bss_end(), paging::READ_WRITE);
            println!("[initialize root table] mapping bss section completed");

            let expected = Some(PhysicalAddr::new(bss_start()));
            let mapped = virt_to_phys(root, VirtualAddr::new(bss_start()));
            assert!(mapped == expected, "expect {:?}, but get {:?}", expected, mapped);


            // map kernel stack`
            println!("[initialize root table] mapping kernel stack...");
            id_map_range(root, kernel_stack_start(), kernel_stack_end() + PAGE_SIZE, paging::READ_WRITE);
            println!("[initialize root table] mapping kernel stack completed");

            let expected = Some(PhysicalAddr::new(kernel_stack_start()));
            let mapped = virt_to_phys(root, VirtualAddr::new(kernel_stack_start()));
            assert!(mapped == expected, "expect {:?}, but get {:?}", expected, mapped);


            // map heap descriptors
            println!("[initialize root table] mapping heap descriptors...");
            id_map_range(root, heap_start(), memory_end(), paging::READ_WRITE);
            println!("[initialize root table] mapping heap descriptors completed");

            let expected = Some(PhysicalAddr::new(heap_start()));
            let mapped = virt_to_phys(root, VirtualAddr::new(heap_start()));
            assert!(mapped == expected, "expect {:?}, but get {:?}", expected, mapped);


            println!("root page table mapping initialized");
        }

        ret
    };
}

pub unsafe fn init() {
    let root = ROOT_TABLE.lock();
    let addr = root.as_ref() as *const _ as usize;
    let ppn = PhysicalAddr::new(addr).extract_ppn_all();

    println!("[paging::init] set satp register, mode: {:?}, ppn: {:#x}", satp::Mode::Sv39, ppn);
    satp::set(satp::Mode::Sv39, 0, ppn);
    println!("[paging::init] set satp register completed");

    println!("[paging::init] sfence_vma_all");
    sfence_vma_all();
    println!("[paging::init] sfence_vma_all completed");
}
