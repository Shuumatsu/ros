use alloc::boxed::Box;
use riscv::{asm::sfence_vma_all, register::satp};
use spin::{Lazy, Mutex};

use crate::memory::layout::{
    bss_end, bss_start, data_end, data_start, heap_start, kernel_stack_end, kernel_stack_start,
    memory_end, rodata_end, rodata_start, text_end, text_start,
};
use crate::platform::{CLINT_BASE, CLINT_SIZE, PLIC_BASE, PLIC_END, UART0_BASE};

pub use paging::sv39::{PAGE_SIZE, PhysicalAddr, PteFlags, Table, VirtualAddr};

/// Sanity-check that `addr` identity-maps to itself in `table`.
fn verify_id_map(table: &Table, addr: usize) {
    let expected = Some(PhysicalAddr::new(addr));
    let mapped = table.translate(VirtualAddr::new(addr));
    assert!(
        mapped == expected,
        "identity map broken at {addr:#x}: expected {expected:?}, got {mapped:?}"
    );
}

/// The kernel's root page table: an identity map of every region the kernel
/// touches, built once on first access. `Box` provides the 4 KiB alignment that
/// [`Table`] requires.
pub static ROOT_TABLE: Lazy<Mutex<Box<Table>>> = Lazy::new(|| {
    let mut table = Box::new(Table::new());
    {
        let t = table.as_mut();

        // Memory-mapped devices.
        t.id_map_range(UART0_BASE, UART0_BASE + PAGE_SIZE, PteFlags::READ_WRITE);
        t.id_map_range(CLINT_BASE, CLINT_BASE + CLINT_SIZE, PteFlags::READ_WRITE);
        t.id_map_range(PLIC_BASE, PLIC_END, PteFlags::READ_WRITE);

        // Kernel image sections, each with its natural permissions.
        t.id_map_range(text_start(), text_end(), PteFlags::READ_EXECUTE);
        t.id_map_range(rodata_start(), rodata_end(), PteFlags::READ);
        t.id_map_range(data_start(), data_end(), PteFlags::READ_WRITE);
        t.id_map_range(bss_start(), bss_end(), PteFlags::READ_WRITE);

        // Kernel stack (with a page of slack) and the heap.
        t.id_map_range(kernel_stack_start(), kernel_stack_end() + PAGE_SIZE, PteFlags::READ_WRITE);
        t.id_map_range(heap_start(), memory_end(), PteFlags::READ_WRITE);

        // Spot-check one address in every region.
        for addr in [
            UART0_BASE,
            CLINT_BASE,
            PLIC_BASE,
            text_start(),
            rodata_start(),
            data_start(),
            bss_start(),
            kernel_stack_start(),
            heap_start(),
        ] {
            verify_id_map(t, addr);
        }
    }
    Mutex::new(table)
});

/// Install the root table into `satp` and enable Sv39 translation.
///
/// # Safety
///
/// Turning on paging reinterprets every subsequent address. The root table
/// must already map the currently executing code, its stack and its data —
/// which [`ROOT_TABLE`] does — or the next instruction fetch will fault.
pub unsafe fn init() {
    let table = ROOT_TABLE.lock();
    let root_pa = PhysicalAddr::new(table.as_ref() as *const Table as usize);
    // SAFETY: the caller guarantees the running kernel stays mapped across the switch.
    unsafe { satp::set(satp::Mode::Sv39, 0, root_pa.ppn()) };
    sfence_vma_all();
}
