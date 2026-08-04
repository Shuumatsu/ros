use core::arch::naked_asm;

use paging::VirtualAddr;
use paging::sv39::PAGE_OFFSET_BITS;

use super::handoff::{CPU_OFFSET, READY_OFFSET, SATP_OFFSET, STACK_TOP_OFFSET};
use crate::memory::boot_table;

unsafe extern "C" {
    #[link_name = "__global_pointer$"]
    static GLOBAL_POINTER: u8;
    static _boot_stack_end: u8;
    static _bss_start: u8;
    static _bss_end: u8;
}

#[unsafe(naked)]
#[unsafe(link_section = ".text.init.entry")]
pub(super) unsafe extern "custom" fn boot_entry() {
    naked_asm!(
        ".option push",
        ".option norvc",
        ".option norelax",
        "li a2, 0",
        "tail {relocate}",
        ".option pop",
        relocate = sym relocate,
    )
}

#[unsafe(naked)]
#[unsafe(link_section = ".text.init.entry")]
unsafe extern "custom" fn secondary_entry() {
    naked_asm!(
        ".option push",
        ".option norvc",
        ".option norelax",
        "li a2, 1",
        "tail {relocate}",
        ".option pop",
        relocate = sym relocate,
    )
}

#[unsafe(naked)]
#[unsafe(link_section = ".text.init.entry")]
unsafe extern "custom" fn relocate() {
    naked_asm!(
        ".option push",
        ".option norvc",
        ".option norelax",

        "lla t0, {trap_park}",
        "csrw stvec, t0",

        "lla t0, {early_table}",
        "srli t0, t0, {page_offset_bits}",
        "li t1, {satp_template}",
        "or t0, t0, t1",
        "csrw satp, t0",
        "sfence.vma",

        "lla t0, 10f",
        "ld t1, 0(t0)",
        "lla t0, 11f",
        "sub a3, t1, t0",
        "jr t1",
        ".balign 8",
        "10: .dword 11f",
        "11:",

        "la gp, {global_pointer}",
        "la t0, {trap_park}",
        "csrw stvec, t0",
        "bnez a2, 20f",

        "la sp, {boot_stack_end}",
        "la t0, {bss_start}",
        "la t1, {bss_end}",
        "bgeu t0, t1, 31f",
        "30:",
        "sd zero, 0(t0)",
        "addi t0, t0, 8",
        "bltu t0, t1, 30b",
        "31:",
        "mv a2, a3",
        "tail {rust_boot}",

        "20:",
        "40:",
        "ld t0, {ready_offset}(a1)",
        "beqz t0, 40b",
        "fence r, rw",
        "ld t0, {satp_offset}(a1)",
        "ld t1, {stack_top_offset}(a1)",
        "ld a1, {cpu_offset}(a1)",
        "csrw satp, t0",
        "sfence.vma",
        "mv sp, t1",
        "tail {rust_secondary}",

        ".option pop",
        trap_park = sym trap_park,
        early_table = sym boot_table::TABLE,
        page_offset_bits = const PAGE_OFFSET_BITS,
        satp_template = const boot_table::SATP_TEMPLATE,
        global_pointer = sym GLOBAL_POINTER,
        boot_stack_end = sym _boot_stack_end,
        bss_start = sym _bss_start,
        bss_end = sym _bss_end,
        ready_offset = const READY_OFFSET,
        satp_offset = const SATP_OFFSET,
        stack_top_offset = const STACK_TOP_OFFSET,
        cpu_offset = const CPU_OFFSET,
        rust_boot = sym crate::start::boot,
        rust_secondary = sym crate::start::secondary,
    )
}

#[unsafe(naked)]
#[unsafe(link_section = ".text.init.trap")]
unsafe extern "custom" fn trap_park() {
    naked_asm!(".option push", ".option norvc", "90:", "wfi", "j 90b", ".option pop",)
}

pub(crate) fn secondary_entry_address() -> VirtualAddr {
    VirtualAddr::new(secondary_entry as *const () as usize)
}
