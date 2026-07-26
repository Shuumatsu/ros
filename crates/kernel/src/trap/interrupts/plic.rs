use core::cmp::max;
use riscv::register::{mie, sie};

use crate::device_tree::{plic_base, uart_irq};
use crate::trap::TrapFrame;
use crate::{print, println};

// Register offsets within the PLIC MMIO region. The *base* comes from the device
// tree; these offsets are fixed by the PLIC spec (the DTB does not — and cannot —
// describe a controller's internal register layout), so they live with the
// driver, not in any platform table.
//   base + 0x000000: source priorities
//   base + 0x001000: pending bits
//   base + 0x002000: enable bits (context 0)
//   base + 0x200000: priority threshold (context 0)
//   base + 0x200004: claim/complete (context 0)
const PLIC_PRIORITY_OFFSET: usize = 0x0;
const PLIC_PENDING_OFFSET: usize = 0x1000;
const PLIC_ENABLE_OFFSET: usize = 0x2000;
const PLIC_THRESHOLD_OFFSET: usize = 0x20_0000;
const PLIC_CLAIM_OFFSET: usize = 0x20_0004;

/// Pointer to the PLIC register `offset` bytes into its MMIO window.
///
/// The single place that turns the device tree's *physical* PLIC base into a
/// usable pointer. It goes through the kernel's direct map, which covers device
/// memory (see [`crate::memory::kernel_table`]) — never the raw physical address,
/// which is only a valid pointer for as long as the boot identity mapping happens
/// to survive.
fn register(offset: usize) -> *mut u32 {
    crate::memory::phys_to_virt(plic_base() + offset) as *mut u32
}

// The platform-level interrupt controller (PLIC) routes all signals through one pin on the CPU--the EI (external interrupt) pin.
// This pin can be enabled via the machine external interrupt enable (meie) bit in the mie register.

// We can configure the PLIC to prioritize interrupt sources or to completely disable some sources, while enabling others.

// https://github.com/riscv/riscv-plic-spec/blob/master/riscv-plic.adoc

// https://osblog.stephenmarz.com/imgs/plic_cpu.png
// https://github.com/qemu/qemu/blob/master/include/hw/riscv/virt.h
unsafe fn enable(intr_id: usize) {
    assert!(intr_id < 1024);

    let bit = 1 << intr_id;
    // 似乎 qemu 是运行在 context 0？
    let ptr = register(PLIC_ENABLE_OFFSET);
    unsafe { ptr.write_volatile(ptr.read_volatile() | bit) };
}

unsafe fn set_priority(intr_id: usize, mut prio: u32) {
    assert!(intr_id < 1024);

    let tsh = {
        let ptr = register(PLIC_THRESHOLD_OFFSET);
        unsafe { ptr.read_volatile() }
    };
    prio = max(prio, tsh);

    let ptr = register(PLIC_PRIORITY_OFFSET);
    unsafe { ptr.add(intr_id).write_volatile(prio) };
}

unsafe fn set_threshold(threshold: u32) {
    let ptr = register(PLIC_THRESHOLD_OFFSET);
    unsafe { ptr.write_volatile(threshold) }
}

/// See if a given interrupt id is pending.
unsafe fn is_pending(intr_id: u32) -> bool {
    let ptr = register(PLIC_PENDING_OFFSET);

    let bits = unsafe { ptr.read_volatile() };
    (1 << intr_id) & bits != 0
}

// returns the ID of the highest priority pending interrupt or zero if there is no pending interrupt
// A successful claim will also atomically clear the corresponding pending bit on the interrupt source.
unsafe fn claim() -> Option<usize> {
    let ptr = register(PLIC_CLAIM_OFFSET);

    match unsafe { ptr.read_volatile() } {
        0 => None,
        intr_id => Some(intr_id as usize),
    }
}

// The PLIC does not check whether the completion ID is the same as the last claim ID for that target.
// If the completion ID does not match an interrupt source that is currently enabled for the target, the completion is silently ignored.
unsafe fn complete(intr_id: usize) {
    let ptr = register(PLIC_CLAIM_OFFSET);
    unsafe { ptr.write_volatile(intr_id as u32) }
}

pub unsafe fn init() {
    println!("enable plic interrupts");
    unsafe { sie::set_sext() };

    unsafe { enable(uart_irq()) };
    // permits all interrupts with non-zero priority
    unsafe { set_threshold(0) };
    unsafe { set_priority(uart_irq(), 1) };
}

pub unsafe fn handler(_tf: &TrapFrame) {
    if let Some(intr_id) = unsafe { claim() } {
        if intr_id == uart_irq() {
            // TODO: real UART RX handling; for now just acknowledge.
            unsafe { complete(intr_id) };
        } else {
            unimplemented!()
        }
    }
}
