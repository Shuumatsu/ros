use alloc::boxed::Box;
use riscv::{asm::sfence_vma_all, register::satp};
use spin::{Lazy, Mutex};

use crate::device_tree;
use crate::memory::layout::{
    bss_end, bss_start, data_end, data_start, heap_start, kernel_stack_end, kernel_stack_start,
    rodata_end, rodata_start, text_end, text_start,
};

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

        // Memory-mapped devices, all discovered from the device tree.
        let uart_base = device_tree::uart_base();
        let clint_base = device_tree::clint_base();
        let plic_base = device_tree::plic_base();
        t.id_map_range(uart_base, uart_base + PAGE_SIZE, PteFlags::READ_WRITE);
        t.id_map_range(clint_base, clint_base + device_tree::clint_size(), PteFlags::READ_WRITE);
        t.id_map_range(plic_base, plic_base + device_tree::plic_size(), PteFlags::READ_WRITE);

        // Kernel image sections, each with its natural permissions.
        t.id_map_range(text_start(), text_end(), PteFlags::READ_EXECUTE);
        t.id_map_range(rodata_start(), rodata_end(), PteFlags::READ);
        t.id_map_range(data_start(), data_end(), PteFlags::READ_WRITE);
        t.id_map_range(bss_start(), bss_end(), PteFlags::READ_WRITE);

        // Kernel stack (with a page of slack) and the heap. The heap runs up to
        // the RAM top the device tree reported, matching `memory::init`.
        let ram_end = crate::device_tree::ram_end()
            .expect("device tree RAM region not discovered before building the page table");
        t.id_map_range(kernel_stack_start(), kernel_stack_end() + PAGE_SIZE, PteFlags::READ_WRITE);
        t.id_map_range(heap_start(), ram_end, PteFlags::READ_WRITE);

        // Spot-check one address in every region.
        for addr in [
            uart_base,
            clint_base,
            plic_base,
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
