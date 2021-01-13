use core::cmp::max;
use riscv::register::{mie, sie};

use crate::trap::TrapFrame;
use crate::{print, println};
// The PLIC is an interrupt controller controlled via MMIO.
use crate::memory::layout::{
    CLAIM_BASE_ADDR, COMPLETE_BASE_ADDR, ENABLE_BASE_ADDR, PENDING_BASE_ADDR, PRIORITY_BASE_ADDR,
    PRIORITY_THRESHOLD_BASE_ADDR,
};

const UART_INTR_ID: usize = 10;

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
    let ptr = ENABLE_BASE_ADDR as *mut u32;
    ptr.write_volatile(ptr.read_volatile() | bit);
}

unsafe fn set_priority(intr_id: usize, mut prio: u32) {
    assert!(intr_id < 1024);

    let tsh = {
        let ptr = PRIORITY_THRESHOLD_BASE_ADDR as *mut u32;
        ptr.read_volatile()
    };
    prio = max(prio, tsh);

    let ptr = PRIORITY_BASE_ADDR as *mut u32;
    ptr.add(intr_id).write_volatile(prio);
}

unsafe fn set_threshold(threshold: u32) {
    let ptr = PRIORITY_THRESHOLD_BASE_ADDR as *mut u32;
    ptr.write_volatile(threshold)
}

/// See if a given interrupt id is pending.
unsafe fn is_pending(intr_id: u32) -> bool {
    let ptr = PENDING_BASE_ADDR as *const u32;

    let bits = ptr.read_volatile();
    (1 << intr_id) & bits != 0
}

// returns the ID of the highest priority pending interrupt or zero if there is no pending interrupt
// A successful claim will also atomically clear the corresponding pending bit on the interrupt source.
unsafe fn claim() -> Option<usize> {
    let ptr = CLAIM_BASE_ADDR as *const u32;

    match ptr.read_volatile() {
        0 => None,
        intr_id => Some(intr_id as usize),
    }
}

// The PLIC does not check whether the completion ID is the same as the last claim ID for that target.
// If the completion ID does not match an interrupt source that is currently enabled for the target, the completion is silently ignored.
unsafe fn complete(intr_id: usize) {
    let ptr = COMPLETE_BASE_ADDR as *mut u32;
    ptr.write_volatile(intr_id as u32)
}

pub unsafe fn init() {
    kprintln!("enable plic interrupts");
    sie::set_sext();

    enable(UART_INTR_ID);
    // permits all interrupts with non-zero priority
    set_threshold(0);
    set_priority(UART_INTR_ID, 1);
}

pub unsafe fn handler(tf: &TrapFrame) {
    if let Some(intr_id) = claim() {
        match intr_id {
            UART_INTR_ID => {
                panic!("qqq");
                complete(UART_INTR_ID);
            }
            _ => {
                unimplemented!()
            }
        }
    }
}
