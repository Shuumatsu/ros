use log::info;
use riscv::asm::sfence_vma_all;
use riscv::register::satp;
use spin::Mutex;

use alloc::alloc::{alloc_zeroed, dealloc, Layout};
use alloc::boxed::Box;
use core::fmt;
use core::mem::size_of;
use core::ptr;

use crate::config::{ENTRIES_PER_PAGE, ENTRY_SIZE, PAGE_SIZE, PAGE_SIZE_BITS};
use crate::memory::addr::{PhysicalAddr, VirtualAddr};
use crate::memory::layout::{
    BSS_END, BSS_START, DATA_END, DATA_START, HEAP_END, HEAP_START, KERNEL_STACK_END,
    KERNEL_STACK_START, RODATA_END, RODATA_START, TEXT_END, TEXT_START,
};
use crate::memory::FRAME_ALLOCATOR;

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
pub struct Entry(usize);
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

impl From<usize> for Entry {
    fn from(bits: usize) -> Self {
        Entry(bits)
    }
}

impl Entry {
    pub fn new(ppn: usize, flags: EntryFlags) -> Self {
        let mut ret = Self::empty();
        ret.set_flags(flags);
        ret.set_ppn(ppn);
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

    pub fn extract_ppn(&self) -> usize {
        extract_range!(self.0, 10, 54)
    }

    pub fn set_ppn(&mut self, frame: usize) {
        self.0 = store_range!(self.0, 10, 54, frame)
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
            if let Some(ppn) = FRAME_ALLOCATOR.lock().alloc(1) {
                self.set_ppn(ppn);
                self.set_flags(flags | EntryFlags::VALID);
            } else {
                panic!("???");
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
    entry.set_ppn(paddr.extract_ppn());
    entry.set_flags(flags | EntryFlags::VALID);
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

    FRAME_ALLOCATOR
        .lock()
        .dealloc(entry.extract_ppn() as usize, 1);
    entry.set_flags(flags & !EntryFlags::VALID);
}

pub unsafe fn free(root: *mut Table) {
    for entry in (*root).entries.iter_mut() {
        let flags = entry.extract_flags();
        if !flags.contains(EntryFlags::VALID) {
            continue;
        }

        let ppn = entry.extract_ppn();
        if entry.is_branch() {
            let table = PhysicalAddr::new(ppn, 0).as_mut_ptr::<Table>();
            free(table);
        } else {
            FRAME_ALLOCATOR
                .lock()
                .dealloc(entry.extract_ppn() as usize, 1);
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

pub unsafe fn map_range<F: Fn(VirtualAddr) -> PhysicalAddr>(
    root: *mut Table,
    start: VirtualAddr,
    end: VirtualAddr,
    flags: EntryFlags,
    f: F,
) {
    let start = align_up!(start.extract_bits(), PAGE_SIZE);
    let end = align_down!(end.extract_bits(), PAGE_SIZE);

    for curr in (start..end).step_by(PAGE_SIZE) {
        let vaddr = VirtualAddr::from(curr);
        let paddr = f(vaddr);
        map(root, vaddr, paddr, flags);
    }
}

pub unsafe fn id_map_range(
    root: *mut Table,
    mut start: VirtualAddr,
    mut end: VirtualAddr,
    flags: EntryFlags,
) {
    map_range(root, start, end, flags, |vaddr| {
        PhysicalAddr::from(vaddr.extract_bits())
    });
}

lazy_static! {
    static ref ROOT_TABLE: Mutex<Box<Table>> = Mutex::new(Box::new(Table::new()));
}

pub fn init() {
    let mut root = ROOT_TABLE.lock();
    unsafe {
        let root = (*root).as_mut() as *mut _;

        info!(
            "[initialize kernel root table] mapping text section [{:#x}, {:#x})...",
            *TEXT_START, *TEXT_END
        );
        id_map_range(
            root,
            VirtualAddr::from(*TEXT_START),
            VirtualAddr::from(*TEXT_END),
            EntryFlags::READ_EXECUTE,
        );
        info!("[initialize kernel root table] mapping text section completed");

        info!(
            "[initialize kernel root table] mapping rodata section [{:#x}, {:#x})...",
            *RODATA_START, *RODATA_END
        );
        id_map_range(
            root,
            VirtualAddr::from(*RODATA_START),
            VirtualAddr::from(*RODATA_END),
            EntryFlags::READ,
        );
        info!("[initialize kernel root table] mapping rodata section completed");

        info!(
            "[initialize kernel root table] mapping data section [{:#x}, {:#x})...",
            *DATA_START, *DATA_END
        );
        id_map_range(
            root,
            VirtualAddr::from(*DATA_START),
            VirtualAddr::from(*DATA_END),
            EntryFlags::READ_WRITE,
        );
        info!("[initialize kernel root table] mapping data section completed");

        info!(
            "[initialize kernel root table] mapping bss section [{:#x}, {:#x})...",
            *BSS_START, *BSS_END
        );
        id_map_range(
            root,
            VirtualAddr::from(*BSS_START),
            VirtualAddr::from(*BSS_END),
            EntryFlags::READ_WRITE,
        );
        info!("[initialize kernel root table] mapping bss section completed");

        info!(
            "[initialize kernel root table] mapping kernel stack [{:#x}, {:#x})...",
            *KERNEL_STACK_START, *KERNEL_STACK_END
        );
        id_map_range(
            root,
            VirtualAddr::from(*KERNEL_STACK_START),
            VirtualAddr::from(*KERNEL_STACK_END),
            EntryFlags::READ_WRITE,
        );
        info!("[initialize kernel root table] mapping kernel stack completed");

        info!(
            "[initialize kernel root table] mapping heap [{:#x}, {:#x})...",
            *HEAP_START, *HEAP_END
        );
        id_map_range(
            root,
            VirtualAddr::from(*HEAP_START),
            VirtualAddr::from(*HEAP_END),
            EntryFlags::READ_WRITE,
        );
        info!("[initialize kernel root table] mapping heap completed");

        println!("root page table mapping completed");
    }

    let addr = root.as_ref() as *const _ as usize;
    let ppn = PhysicalAddr::from(addr).extract_ppn();

    unsafe {
        // satp::set(satp::Mode::Sv39, 0, ppn);
        // println!("[paging::init] set satp register completed");

        // sfence_vma_all();

        // println!("[paging::init] virtual memory initialized");
    }
}
