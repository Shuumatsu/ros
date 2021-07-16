use alloc::boxed::Box;

use spin::Mutex;

use alloc::alloc::{alloc_zeroed, dealloc, Layout};
use core::fmt;
use core::mem::size_of;
use core::ptr;

use crate::config::{ENTRIES_PER_PAGE, ENTRY_SIZE, PAGE_SIZE, PAGE_SIZE_BITS};
use crate::memory::FRAME_ALLOCATOR;

use crate::memory::addr::{PhysicalAddr, VirtualAddr};

// +----------+---------+---------+---------+-------+---+---+---+---+---+---+---+---+
// | Not Used | PPN[2]  | PPN[1]  | PPN[0]  | RSW   | D | A | G | U | X | W | R | V |
// +----------+---------+---------+---------+-------+---+---+---+---+---+---+---+---+
// | 63 - 54  | 53 - 28 | 27 - 19 | 18 - 10 | 9 - 8 | 7 | 6 | 5 | 4 | 3 | 2 | 1 | 0 |
// +----------+---------+---------+---------+-------+---+---+---+---+---+---+---+---+
bitflags! {
    pub struct EntryFlags: u8 {
        const VALID     = 1 << 0;
        const READ      = 1 << 1;
        const WRITE     = 1 << 2;
        const EXECUTE   = 1 << 3;
        const USER      = 1 << 4;
        const GLOBAL    = 1 << 5;
        const ACCESS    = 1 << 6;
        const DIRTY     = 1 << 7;

        const READ_WRITE = Self::READ.bits | Self::WRITE.bits;
        const READ_EXECUTE = Self::READ.bits | Self::EXECUTE.bits;
        const READ_WRITE_EXECUTE = Self::READ.bits | Self::WRITE.bits | Self::EXECUTE.bits;

        const USER_READ_WRITE = Self::READ_WRITE.bits | Self::USER.bits;
        const USER_READ_EXECUTE = Self::READ_EXECUTE.bits | Self::USER.bits;
        const USER_READ_WRITE_EXECUTE = Self::READ_WRITE_EXECUTE.bits | Self::USER.bits;
  }
}

#[derive(Copy, Clone, Default)]
#[repr(transparent)]
pub struct Entry(u64);
const_assert_eq!(size_of::<Entry>(), ENTRY_SIZE);

unsafe impl Send for Entry {}

impl fmt::Debug for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!(
            "Entry({:#x} = {:#x}, {:?})",
            self.0,
            self.extract_ppn(),
            self.extract_flags()
        ))
    }
}

impl From<u64> for Entry {
    fn from(bits: u64) -> Self {
        Entry(bits)
    }
}

impl Entry {
    pub fn new(paddr: PhysicalAddr, flags: EntryFlags) -> Self {
        let mut ret = Self::empty();
        ret.set_flags(flags);
        ret.set_page(paddr);
        ret
    }

    pub fn empty() -> Self {
        Self::default()
    }

    pub fn extract_flags(&self) -> EntryFlags {
        EntryFlags::from_bits_truncate(self.0 as u8)
    }

    pub fn set_flags(&mut self, flags: EntryFlags) {
        self.0 = store_range!(self.0, 0, 8, flags.bits());
    }

    pub fn extract_ppn(&self) -> u64 {
        extract_range!(self.0, 10, 54)
    }

    pub fn set_frame(&mut self, frame: u64) {
        self.0 = store_range!(self.0, 10, 54, frame)
    }
    pub fn set_page(&mut self, paddr: PhysicalAddr) {
        self.set_frame(paddr.extract_ppn())
    }

    // A leaf has one or more RWX bits set
    pub fn is_leaf(&self) -> bool {
        let flags = self.extract_flags();
        flags.intersects(EntryFlags::READ_WRITE_EXECUTE)
    }

    pub fn is_branch(&self) -> bool {
        !self.is_leaf()
    }

    unsafe fn ensure(&mut self) -> bool {
        let flags = self.extract_flags();
        if !flags.contains(EntryFlags::VALID) {
            if let Some(ppn) = FRAME_ALLOCATOR.lock().alloc() {
                self.set_frame(ppn as u64);
                self.set_flags(flags | EntryFlags::VALID);
            } else {
                return false;
            }
        }
        true
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
pub struct Table {
    entries: [Entry; ENTRIES_PER_PAGE],
}
const_assert_eq!(size_of::<Table>(), PAGE_SIZE);

unsafe impl Send for Table {}

impl Table {
    pub fn new() -> Self {
        Table {
            entries: [Entry::from(0); ENTRIES_PER_PAGE],
        }
    }
}

// The bits should contain only the following: Read, Write, Execute, User, and/or Global
// The bits MUST include one or more of the following: Read, Write, Execute
pub unsafe fn map(
    root: *mut Table,
    vaddr: VirtualAddr,
    paddr: PhysicalAddr,
    flags: EntryFlags,
) -> bool {
    let mut table = root;
    {
        let entry = &mut (*table).entries[vaddr.extract_vpn(2) as usize];
        if !entry.ensure() {
            return false;
        }

        let addr = PhysicalAddr::new(entry.extract_ppn(), 0);
        table = addr.as_mut_ptr();
    }
    {
        let entry = &mut (*table).entries[vaddr.extract_vpn(1) as usize];
        if !entry.ensure() {
            return false;
        }

        let addr = PhysicalAddr::new(entry.extract_ppn(), 0);
        table = addr.as_mut_ptr();
    }

    let entry = &mut (*table).entries[vaddr.extract_vpn(0) as usize];
    if !entry.ensure() {
        return false;
    }

    entry.set_page(paddr);
    entry.set_flags(flags);

    true
}

pub unsafe fn unmap(root: *mut Table, vaddr: VirtualAddr) {
    let mut table = root;
    {
        let entry = &mut (*table).entries[vaddr.extract_vpn(2) as usize];
        if !entry.extract_flags().contains(EntryFlags::VALID) {
            return;
        }

        let addr = PhysicalAddr::new(entry.extract_ppn(), 0);
        table = addr.as_mut_ptr();
    }
    {
        let entry = &mut (*table).entries[vaddr.extract_vpn(1) as usize];
        if !entry.extract_flags().contains(EntryFlags::VALID) {
            return;
        }

        let addr = PhysicalAddr::new(entry.extract_ppn(), 0);
        table = addr.as_mut_ptr();
    }

    let entry = &mut (*table).entries[vaddr.extract_vpn(0) as usize];
    let flags = entry.extract_flags();
    if !flags.contains(EntryFlags::VALID) {
        return;
    }

    FRAME_ALLOCATOR.lock().dealloc(entry.extract_ppn());
    entry.set_flags(flags & !EntryFlags::VALID);
}

pub unsafe fn free(root: *mut Table) {
    for entry in (*root).entries.iter_mut() {
        let flags = entry.extract_flags();
        if flags.contains(EntryFlags::VALID) {
            continue;
        }

        let ppn = entry.extract_ppn();
        if entry.is_branch() {
            let table = PhysicalAddr::new(ppn, 0).as_mut_ptr::<Table>();
            free(table);
        } else {
            FRAME_ALLOCATOR.lock().dealloc(entry.extract_ppn());
        }
        entry.set_flags(flags & !EntryFlags::VALID);
    }
}

pub fn virt_to_phys(root: *const Table, vaddr: VirtualAddr) -> Option<PhysicalAddr> {
    let mut table = root;
    {
        let entry = &mut (unsafe { *table }).entries[vaddr.extract_vpn(2) as usize];
        if !entry.extract_flags().contains(EntryFlags::VALID) {
            return None;
        }

        let addr = PhysicalAddr::new(entry.extract_ppn(), 0);
        table = addr.as_mut_ptr();
    }
    {
        let entry = &mut (unsafe { *table }).entries[vaddr.extract_vpn(1) as usize];
        if !entry.extract_flags().contains(EntryFlags::VALID) {
            return None;
        }

        let addr = PhysicalAddr::new(entry.extract_ppn(), 0);
        table = addr.as_mut_ptr();
    }

    let entry = &mut (unsafe { *table }).entries[vaddr.extract_vpn(0) as usize];
    if !entry.extract_flags().contains(EntryFlags::VALID) {
        return None;
    }

    Some(PhysicalAddr::new(
        entry.extract_ppn(),
        vaddr.extract_offset(),
    ))
}

// lazy_static! {
//     pub static ref ROOT_TABLE: Mutex<Box<Table>> = unsafe {
//         let ret = Mutex::new(Box::new(Table::new()));

// {
//     let root = ret.lock().as_mut() as *mut _;
//     println!("[initialize root table] root page table created at, {:?}", root);

//     // UART
//     println!("[initialize root table] mapping UART...");
//     id_map_range(root, UART_BASE_ADDR, UART_BASE_ADDR + PAGE_SIZE, paging::READ_WRITE);
//     println!("[initialize root table] mapping UART completed");

//     let expected = Some(PhysicalAddr::new(UART_BASE_ADDR));
//     let mapped = virt_to_phys(root, VirtualAddr::new(UART_BASE_ADDR));
//     assert!(mapped == expected, "expect {:?}, but get {:?}", expected, mapped);

//     // CLINT
//     println!("[initialize root table] mapping CLINT...");
//     id_map_range(root, CLINT_BASE_ADDR, CLINT_BASE_ADDR + PAGE_SIZE, paging::READ_WRITE);
//     println!("[initialize root table] mapping CLINT completed");

//     let expected = Some(PhysicalAddr::new(CLINT_BASE_ADDR));
//     let mapped = virt_to_phys(root, VirtualAddr::new(CLINT_BASE_ADDR));
//     assert!(mapped == expected, "expect {:?}, but get {:?}", expected, mapped);

//     // PLIC
//     println!("[initialize root table] mapping PLIC...");
//     id_map_range(root, PLIC_BASE_ADDR, PLIC_END_ADDR, paging::READ_WRITE);
//     println!("[initialize root table] mapping PLIC completed");

//     let expected = Some(PhysicalAddr::new(PLIC_BASE_ADDR));
//     let mapped = virt_to_phys(root, VirtualAddr::new(PLIC_BASE_ADDR));
//     assert!(mapped == expected, "expect {:?}, but get {:?}", expected, mapped);

//     // map text section
//     println!("[initialize root table] mapping text section...");
//     id_map_range(root, text_start(), text_end(), paging::READ_EXECUTE);
//     println!("[initialize root table] mapping text section completed");

//     let expected = Some(PhysicalAddr::new(text_start()));
//     let mapped = virt_to_phys(root, VirtualAddr::new(text_start()));
//     assert!(mapped == expected, "expect {:?}, but get {:?}", expected, mapped);

//     // map rodata section
//     println!("[initialize root table] mapping rodata section...");
//     id_map_range(root, rodata_start(), rodata_end(), paging::READ);
//     println!("[initialize root table] mapping rodata section completed");

//     let expected = Some(PhysicalAddr::new(rodata_start()));
//     let mapped = virt_to_phys(root, VirtualAddr::new(rodata_start()));
//     assert!(mapped == expected, "expect {:?}, but get {:?}", expected, mapped);

//     // map data section
//     println!("[initialize root table] mapping data section...");
//     id_map_range(root, data_start(), data_end(), paging::READ_WRITE);
//     println!("[initialize root table] mapping data section completed");

//     let expected = Some(PhysicalAddr::new(data_start()));
//     let mapped = virt_to_phys(root, VirtualAddr::new(data_start()));
//     assert!(mapped == expected, "expect {:?}, but get {:?}", expected, mapped);

//     // map bss section
//     println!("[initialize root table] mapping bss section...");
//     id_map_range(root, bss_start(), bss_end(), paging::READ_WRITE);
//     println!("[initialize root table] mapping bss section completed");

//     let expected = Some(PhysicalAddr::new(bss_start()));
//     let mapped = virt_to_phys(root, VirtualAddr::new(bss_start()));
//     assert!(mapped == expected, "expect {:?}, but get {:?}", expected, mapped);

//     // map kernel stack`
//     println!("[initialize root table] mapping kernel stack...");
//     id_map_range(root, kernel_stack_start(), kernel_stack_end() + PAGE_SIZE, paging::READ_WRITE);
//     println!("[initialize root table] mapping kernel stack completed");

//     let expected = Some(PhysicalAddr::new(kernel_stack_start()));
//     let mapped = virt_to_phys(root, VirtualAddr::new(kernel_stack_start()));
//     assert!(mapped == expected, "expect {:?}, but get {:?}", expected, mapped);

//     // map heap descriptors
//     println!("[initialize root table] mapping heap descriptors...");
//     id_map_range(root, heap_start(), memory_end(), paging::READ_WRITE);
//     println!("[initialize root table] mapping heap descriptors completed");

//     let expected = Some(PhysicalAddr::new(heap_start()));
//     let mapped = virt_to_phys(root, VirtualAddr::new(heap_start()));
//     assert!(mapped == expected, "expect {:?}, but get {:?}", expected, mapped);

//     println!("root page table mapping initialized");
// }

//         ret
//     };
// }

pub unsafe fn init() {
    // let root = ROOT_TABLE.lock();
    // let addr = root.as_ref() as *const _ as usize;
    // let ppn = PhysicalAddr::new(addr).extract_ppn_all();

    // println!(
    //     "[paging::init] set satp register, mode: {:?}, ppn: {:#x}",
    //     satp::Mode::Sv39,
    //     ppn
    // );
    // satp::set(satp::Mode::Sv39, 0, ppn);
    // println!("[paging::init] set satp register completed");

    // println!("[paging::init] sfence_vma_all");
    // sfence_vma_all();
    // println!("[paging::init] sfence_vma_all completed");
}
