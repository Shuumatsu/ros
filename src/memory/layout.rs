extern "C" {
    static _text_start: usize;
}
#[inline]
pub fn text_start() -> usize { unsafe { &_text_start as *const _ as _ } }

extern "C" {
    static _text_end: usize;
}
#[inline]
pub fn text_end() -> usize { unsafe { &_text_end as *const _ as _ } }

extern "C" {
    static _global_pointer: usize;
}
#[inline]
pub fn global_pointer() -> usize { unsafe { &_global_pointer as *const _ as _ } }

extern "C" {
    static _rodata_start: usize;
}
#[inline]
pub fn rodata_start() -> usize { unsafe { &_rodata_start as *const _ as _ } }

extern "C" {
    static _rodata_end: usize;
}
#[inline]
pub fn rodata_end() -> usize { unsafe { &_rodata_end as *const _ as _ } }

extern "C" {
    static _data_start: usize;
}
#[inline]
pub fn data_start() -> usize { unsafe { &_data_start as *const _ as _ } }

extern "C" {
    static _data_end: usize;
}
#[inline]
pub fn data_end() -> usize { unsafe { &_data_end as *const _ as _ } }

extern "C" {
    static _bss_start: usize;
}
#[inline]
pub fn bss_start() -> usize { unsafe { &_bss_start as *const _ as _ } }

extern "C" {
    static _bss_end: usize;
}
#[inline]
pub fn bss_end() -> usize { unsafe { &_bss_end as *const _ as _ } }

extern "C" {
    static _memory_start: usize;
}

extern "C" {
    static _kernel_stack_start: usize;
}
#[inline]
pub fn kernel_stack_start() -> usize { unsafe { &_kernel_stack_start as *const _ as _ } }

extern "C" {
    static _kernel_stack_end: usize;
}
#[inline]
pub fn kernel_stack_end() -> usize { unsafe { &_kernel_stack_end as *const _ as _ } }

extern "C" {
    static _heap_start: usize;
}
#[inline]
pub fn heap_start() -> usize { unsafe { &_heap_start as *const _ as _ } }

extern "C" {
    static _heap_size: usize;
}
#[inline]
pub fn heap_size() -> usize { unsafe { &_heap_size as *const _ as _ } }

#[inline]
pub fn memory_start() -> usize { unsafe { &_memory_start as *const _ as _ } }

extern "C" {
    static _memory_end: usize;
}
#[inline]
pub fn memory_end() -> usize { unsafe { &_memory_end as *const _ as _ } }

// ---

// PLIC Memory Map:
// base + 0x000000: Reserved (interrupt source 0 does not exist)
// base + 0x000004: Interrupt source 1 priority
// base + 0x000008: Interrupt source 2 priority
// ...
// base + 0x000FFC: Interrupt source 1023 priority
// base + 0x001000: Interrupt Pending bit 0-31
// base + 0x00107C: Interrupt Pending bit 992-1023
// ...
// base + 0x002000: Enable bits for sources 0-31 on context 0
// base + 0x002004: Enable bits for sources 32-63 on context 0
// ...
// base + 0x00207F: Enable bits for sources 992-1023 on context 0
// base + 0x002080: Enable bits for sources 0-31 on context 1
// base + 0x002084: Enable bits for sources 32-63 on context 1
// ...
// base + 0x0020FF: Enable bits for sources 992-1023 on context 1
// base + 0x002100: Enable bits for sources 0-31 on context 2
// base + 0x002104: Enable bits for sources 32-63 on context 2
// ...
// base + 0x00217F: Enable bits for sources 992-1023 on context 2
// ...
// base + 0x1F1F80: Enable bits for sources 0-31 on context 15871
// base + 0x1F1F84: Enable bits for sources 32-63 on context 15871
// base + 0x1F1FFF: Enable bits for sources 992-1023 on context 15871
// ...
// base + 0x1FFFFC: Reserved
// base + 0x200000: Priority threshold for context 0
// base + 0x200004: Claim/complete for context 0
// base + 0x200008: Reserved
// ...
// base + 0x200FFC: Reserved
// base + 0x201000: Priority threshold for context 1
// base + 0x201004: Claim/complete for context 1
// ...
// base + 0x3FFE000: Priority threshold for context 15871
// base + 0x3FFE004: Claim/complete for context 15871
// base + 0x3FFE008: Reserved
// ...
// base + 0x3FFFFFC: Reserved

pub const PLIC_BASE_ADDR: usize = 0x0c00_0000;
pub const PLIC_END_ADDR: usize = PLIC_BASE_ADDR + 0x3FFFFFC;

// The interrupt priority for each interrupt source.
pub const PRIORITY_BASE_ADDR: usize = PLIC_BASE_ADDR;
// The interrupt pending status of each interrupt source.
pub const PENDING_BASE_ADDR: usize = PLIC_BASE_ADDR + 0x1000;
// The enablement of interrupt source of each context.
pub const ENABLE_BASE_ADDR: usize = PLIC_BASE_ADDR + 0x2000;
// The interrupt priority threshold of each context.
// The PLIC will mask all PLIC interrupts of a priority less than or equal to threshold.
// For example, a`threshold` value of zero permits all interrupts with non-zero priority.
pub const PRIORITY_THRESHOLD_BASE_ADDR: usize = PLIC_BASE_ADDR + 0x20_0000;
// The register to acquire interrupt source ID of each context.
pub const CLAIM_BASE_ADDR: usize = PLIC_BASE_ADDR + 0x20_0004;
// The register to send interrupt completion message to the associated gateway.
pub const COMPLETE_BASE_ADDR: usize = PLIC_BASE_ADDR + 0x20_0004;

// ---

pub const CLINT_BASE_ADDR: usize = 0x0200_0000;
pub const CLINT_SIZE: usize = 0x0001_0000;

pub fn clint_mtimecmp(hartid: usize) -> usize { CLINT_BASE_ADDR + 0x4000 + 8 * (hartid) }
// cycles since boot.
pub fn clint_mtime(hartid: usize) -> usize { CLINT_BASE_ADDR + 0xBFF8 }

// ---

// QEMU emulates the NS16550A UART chipset.
pub const UART_BASE_ADDR: usize = 0x1000_0000;

// ---

// static const struct MemmapEntry {
//     hwaddr base;
//     hwaddr size;
// } virt_memmap[] = {
//     [VIRT_DEBUG] =       {        0x0,         0x100 },
//     [VIRT_MROM] =        {     0x1000,        0xf000 },
//     [VIRT_TEST] =        {   0x100000,        0x1000 },
//     [VIRT_RTC] =         {   0x101000,        0x1000 },
//     [VIRT_CLINT] =       {  0x2000000,       0x10000 },
//     [VIRT_PCIE_PIO] =    {  0x3000000,       0x10000 },
//     [VIRT_PLIC] =        {  0xc000000, VIRT_PLIC_SIZE(VIRT_CPUS_MAX * 2) },
//     [VIRT_UART0] =       { 0x10000000,         0x100 },
//     [VIRT_VIRTIO] =      { 0x10001000,        0x1000 },
//     [VIRT_FLASH] =       { 0x20000000,     0x4000000 },
//     [VIRT_PCIE_ECAM] =   { 0x30000000,    0x10000000 },
//     [VIRT_PCIE_MMIO] =   { 0x40000000,    0x40000000 },
//     [VIRT_DRAM] =        { 0x80000000,           0x0 },
// };
