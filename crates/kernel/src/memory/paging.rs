use alloc::boxed::Box;
use riscv::{asm::sfence_vma_all, register::satp};
use spin::{Lazy, Mutex};

use crate::device_tree;
use crate::memory::layout::{
    bss_end, bss_start, data_end, data_start, heap_start, kernel_stack_end, kernel_stack_start,
    rodata_end, rodata_start, text_end, text_start,
};

pub use paging::sv39::{PAGE_SIZE, PhysicalAddr, PteFlags, Table, VirtualAddr};

/// Base of the kernel's high-half alias: the bottom of the Sv39 high canonical
/// half. A kernel virtual address equals its physical address plus
/// [`KERNEL_VA_OFFSET`].
pub const KERNEL_VA_BASE: usize = 0xffff_ffc0_0000_0000;
/// High VA = physical address + this offset (`KERNEL_VA_BASE - DRAM_BASE`).
pub const KERNEL_VA_OFFSET: usize = KERNEL_VA_BASE - crate::platform::DRAM_BASE;

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

        // High-half alias of the kernel image: the SAME physical frames are also
        // reachable at KERNEL_VA_BASE+, so the PC can be moved into the high half
        // once paging is on (see `verify_high_half`). Physical target of a high
        // VA is `va - KERNEL_VA_OFFSET`.
        let to_phys = |v: VirtualAddr| PhysicalAddr::new(v.bits().wrapping_sub(KERNEL_VA_OFFSET));
        let hi = |lo: usize| lo.wrapping_add(KERNEL_VA_OFFSET);
        t.map_range(hi(text_start()), hi(text_end()), to_phys, PteFlags::READ_EXECUTE);
        t.map_range(hi(rodata_start()), hi(rodata_end()), to_phys, PteFlags::READ);
        t.map_range(hi(data_start()), hi(data_end()), to_phys, PteFlags::READ_WRITE);
        t.map_range(hi(bss_start()), hi(bss_end()), to_phys, PteFlags::READ_WRITE);
        t.map_range(
            hi(kernel_stack_start()),
            hi(kernel_stack_end() + PAGE_SIZE),
            to_phys,
            PteFlags::READ_WRITE,
        );

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

        // The high-half alias must resolve back to the physical frame it shadows.
        assert_eq!(
            t.translate(VirtualAddr::new(hi(text_start()))),
            Some(PhysicalAddr::new(text_start())),
            "high-half alias of text_start does not map to its physical frame"
        );
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

/// Transition smoke test (plan B): turn on Sv39 with the identity + high-half
/// alias table, jump the PC into the high-half alias to prove the same code runs
/// there, then turn paging back off.
///
/// It deliberately does NOT leave the higher-half kernel installed — that needs
/// the kernel linked at high VAs — it only proves the alias-map + jump mechanism
/// works. The identity map keeps the current (low-linked) PC, stack and data
/// valid across the `satp` flip; PC-relative calls made from the high alias
/// resolve to high aliases too, which are mapped, while device MMIO is reached
/// through the identity map.
///
/// # Safety
/// Must run in S-mode after the heap is up (`ROOT_TABLE` allocates its frames on
/// the heap). Restores Bare mode before returning, so the rest of boot continues
/// with physical addressing unchanged.
pub unsafe fn verify_high_half() {
    unsafe { init() };

    let low_pc: usize;
    unsafe { core::arch::asm!("auipc {}, 0", out(reg) low_pc) };
    println!("[paging] Sv39 on; still at low VA {:#x} via identity map", low_pc);

    // Jump into the high-half alias of `high_half_probe` and run it there.
    let probe_hi = (high_half_probe as usize).wrapping_add(KERNEL_VA_OFFSET);
    let probe: extern "C" fn() = unsafe { core::mem::transmute(probe_hi) };
    probe();

    // Restore bare (physical) addressing so the rest of boot — linked and
    // running at low addresses — continues unchanged.
    unsafe {
        satp::set(satp::Mode::Bare, 0, 0);
        sfence_vma_all();
    }
    println!("[paging] Sv39 off; back to bare physical addressing");
}

/// Runs from the kernel's high-half alias (see [`verify_high_half`]); prints its
/// own PC, which must be `>= KERNEL_VA_BASE`.
#[inline(never)]
extern "C" fn high_half_probe() {
    let pc: usize;
    unsafe { core::arch::asm!("auipc {}, 0", out(reg) pc) };
    println!("[paging] >>> executing at HIGH-HALF VA {:#x} <<<", pc);
}
