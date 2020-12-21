use alloc::boxed::Box;
use lazy_static::lazy_static;
use riscv::{asm::sfence_vma_all, register::satp};
use spin::Mutex;

pub use crate::arch::riscv64::paging::sv39::{
    virt_to_phys, PhysicalAddr, Table, VirtualAddr, PAGE_SIZE,
};
use crate::interrupt::CLINT_BASE_ADDR;
use crate::memory::layout::{
    bss_end, bss_start, data_end, data_start, heap_size, heap_start, kernel_stack_end,
    kernel_stack_start, rodata_end, rodata_start, text_end, text_start,
};
use crate::uart::UART_BASE_ADDR;
use crate::utils::{align_down, align_up};
use crate::{arch::riscv64::paging::sv39 as paging, memory::layout::memory_end};
use crate::{kprint, kprintln};

pub unsafe fn id_map_range(root: *mut Table, mut start: usize, mut end: usize, bits: usize) {
    start = align_up(start, PAGE_SIZE);
    end = align_down(end, PAGE_SIZE);

    kprintln!(
        "[id_map_range] root: {:?}, start: {:#x}, end: {:#x}, bits: {:#b}",
        root,
        start,
        end,
        bits
    );

    for curr in (start..end).step_by(PAGE_SIZE) {
        let vaddr = VirtualAddr::new(curr);
        let paddr = PhysicalAddr::new(curr);
        paging::map(root, vaddr, paddr, bits);
    }
    kprintln!(
        "[id_map_range completed] root: {:?}, start: {:#x}, end: {:#x}, bits: {:#b}",
        root,
        start,
        end,
        bits
    );
}

lazy_static! {
    pub static ref ROOT_TABLE: Mutex<Box<Table>> = unsafe {
        let ret = Mutex::new(Box::new(Table::new()));

        {
            let root = ret.lock().as_mut() as *mut _;
            kprintln!("[initialize root table] root page table created at, {:?}", root);


            // UART
            kprintln!("[initialize root table] mapping UART...");
            id_map_range(root, UART_BASE_ADDR, UART_BASE_ADDR + PAGE_SIZE, paging::READ_WRITE);
            kprintln!("[initialize root table] mapping UART completed");

            let expected = Some(PhysicalAddr::new(UART_BASE_ADDR));
            let mapped = virt_to_phys(root, VirtualAddr::new(UART_BASE_ADDR));
            assert!(mapped == expected, "expect {:?}, but get {:?}", expected, mapped);


            // CLINT
            kprintln!("[initialize root table] mapping CLINT...");
            id_map_range(root, CLINT_BASE_ADDR, CLINT_BASE_ADDR + PAGE_SIZE, paging::READ_WRITE);
            kprintln!("[initialize root table] mapping CLINT completed");

            let expected = Some(PhysicalAddr::new(CLINT_BASE_ADDR));
            let mapped = virt_to_phys(root, VirtualAddr::new(CLINT_BASE_ADDR));
            assert!(mapped == expected, "expect {:?}, but get {:?}", expected, mapped);


            // map text section
            kprintln!("[initialize root table] mapping text section...");
            id_map_range(root, text_start(), text_end(), paging::READ_EXECUTE);
            kprintln!("[initialize root table] mapping text section completed");

            let expected = Some(PhysicalAddr::new(text_start()));
            let mapped = virt_to_phys(root, VirtualAddr::new(text_start()));
            assert!(mapped == expected, "expect {:?}, but get {:?}", expected, mapped);


            // map rodata section
            kprintln!("[initialize root table] mapping rodata section...");
            id_map_range(root, rodata_start(), rodata_end(), paging::READ);
            kprintln!("[initialize root table] mapping rodata section completed");

            let expected = Some(PhysicalAddr::new(rodata_start()));
            let mapped = virt_to_phys(root, VirtualAddr::new(rodata_start()));
            assert!(mapped == expected, "expect {:?}, but get {:?}", expected, mapped);


            // map data section
            kprintln!("[initialize root table] mapping data section...");
            id_map_range(root, data_start(), data_end(), paging::READ_WRITE);
            kprintln!("[initialize root table] mapping data section completed");

            let expected = Some(PhysicalAddr::new(data_start()));
            let mapped = virt_to_phys(root, VirtualAddr::new(data_start()));
            assert!(mapped == expected, "expect {:?}, but get {:?}", expected, mapped);


            // map bss section
            kprintln!("[initialize root table] mapping bss section...");
            id_map_range(root, bss_start(), bss_end(), paging::READ_WRITE);
            kprintln!("[initialize root table] mapping bss section completed");

            let expected = Some(PhysicalAddr::new(bss_start()));
            let mapped = virt_to_phys(root, VirtualAddr::new(bss_start()));
            assert!(mapped == expected, "expect {:?}, but get {:?}", expected, mapped);


            // map kernel stack`
            kprintln!("[initialize root table] mapping kernel stack...");
            id_map_range(root, kernel_stack_start(), kernel_stack_end() + PAGE_SIZE, paging::READ_WRITE);
            kprintln!("[initialize root table] mapping kernel stack completed");

            let expected = Some(PhysicalAddr::new(kernel_stack_start()));
            let mapped = virt_to_phys(root, VirtualAddr::new(kernel_stack_start()));
            assert!(mapped == expected, "expect {:?}, but get {:?}", expected, mapped);


            // map heap descriptors
            kprintln!("[initialize root table] mapping heap descriptors...");
            id_map_range(root, heap_start(), memory_end(), paging::READ_WRITE);
            kprintln!("[initialize root table] mapping heap descriptors completed");

            let expected = Some(PhysicalAddr::new(heap_start()));
            let mapped = virt_to_phys(root, VirtualAddr::new(heap_start()));
            assert!(mapped == expected, "expect {:?}, but get {:?}", expected, mapped);

            kprintln!("root page table mapping initialized");
        }

        ret
    };
}


pub unsafe fn init() {
    let root = ROOT_TABLE.lock();
    let addr = root.as_ref() as *const _ as usize;
    let ppn = PhysicalAddr::new(addr).extract_ppn_all();

    kprintln!("[paging::init] set satp register, mode: {:?}, ppn: {:#x}", satp::Mode::Sv39, ppn);
    satp::set(satp::Mode::Sv39, 0, ppn);
    kprintln!("[paging::init] set satp register completed");

    kprintln!("[paging::init] sfence_vma_all");
    sfence_vma_all();
    kprintln!("[paging::init] sfence_vma_all completed");
}
