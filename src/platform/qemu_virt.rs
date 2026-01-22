//! Hardware constants for QEMU RISC-V virt machine.
//!
//! Reference: https://github.com/qemu/qemu/blob/master/hw/riscv/virt.c
//!
//! Memory map:
//! ```text
//! [VIRT_DEBUG]     = { 0x0,        0x100 }
//! [VIRT_MROM]      = { 0x1000,     0xf000 }
//! [VIRT_TEST]      = { 0x100000,   0x1000 }
//! [VIRT_RTC]       = { 0x101000,   0x1000 }
//! [VIRT_CLINT]     = { 0x2000000,  0x10000 }
//! [VIRT_PCIE_PIO]  = { 0x3000000,  0x10000 }
//! [VIRT_PLIC]      = { 0xc000000,  0x600000 }
//! [VIRT_UART0]     = { 0x10000000, 0x100 }
//! [VIRT_VIRTIO]    = { 0x10001000, 0x1000 }
//! [VIRT_FLASH]     = { 0x20000000, 0x4000000 }
//! [VIRT_PCIE_ECAM] = { 0x30000000, 0x10000000 }
//! [VIRT_PCIE_MMIO] = { 0x40000000, 0x40000000 }
//! [VIRT_DRAM]      = { 0x80000000, ... }
//! ```

// ============================================================================
// UART (NS16550A)
// ============================================================================

pub const UART0_BASE: usize = 0x1000_0000;
pub const UART0_SIZE: usize = 0x100;
pub const UART0_IRQ: usize = 10;

// ============================================================================
// CLINT (Core Local Interruptor)
// ============================================================================

pub const CLINT_BASE: usize = 0x0200_0000;
pub const CLINT_SIZE: usize = 0x0001_0000;

/// Returns the address of mtimecmp register for a given hart.
#[inline]
pub fn clint_mtimecmp(hartid: usize) -> usize {
    CLINT_BASE + 0x4000 + 8 * hartid
}

/// Machine time register - cycles since boot.
pub const CLINT_MTIME: usize = CLINT_BASE + 0xBFF8;

// ============================================================================
// PLIC (Platform-Level Interrupt Controller)
// ============================================================================
//
// Memory map:
//   base + 0x000000: Reserved (interrupt source 0 does not exist)
//   base + 0x000004: Interrupt source 1 priority
//   ...
//   base + 0x000FFC: Interrupt source 1023 priority
//   base + 0x001000: Interrupt pending bit 0-31
//   ...
//   base + 0x001FFF: Interrupt pending bits
//   base + 0x002000: Enable bits for sources 0-31 on context 0
//   ...
//   base + 0x1FFFFF: Enable bits
//   base + 0x200000: Priority threshold for context 0
//   base + 0x200004: Claim/complete for context 0
//   ...

pub const PLIC_BASE: usize = 0x0c00_0000;
pub const PLIC_SIZE: usize = 0x0060_0000;
pub const PLIC_END: usize = PLIC_BASE + PLIC_SIZE;

/// Interrupt priority for each source (offset from PLIC_BASE).
pub const PLIC_PRIORITY_OFFSET: usize = 0x0;
/// Interrupt pending status (offset from PLIC_BASE).
pub const PLIC_PENDING_OFFSET: usize = 0x1000;
/// Interrupt enable bits per context (offset from PLIC_BASE).
pub const PLIC_ENABLE_OFFSET: usize = 0x2000;
/// Priority threshold per context (offset from PLIC_BASE).
pub const PLIC_THRESHOLD_OFFSET: usize = 0x20_0000;
/// Claim/complete register per context (offset from PLIC_BASE).
pub const PLIC_CLAIM_OFFSET: usize = 0x20_0004;

/// Interrupt priority base address.
pub const PLIC_PRIORITY_BASE: usize = PLIC_BASE + PLIC_PRIORITY_OFFSET;
/// Interrupt pending base address.
pub const PLIC_PENDING_BASE: usize = PLIC_BASE + PLIC_PENDING_OFFSET;
/// Interrupt enable base address.
pub const PLIC_ENABLE_BASE: usize = PLIC_BASE + PLIC_ENABLE_OFFSET;
/// Priority threshold base address.
pub const PLIC_THRESHOLD_BASE: usize = PLIC_BASE + PLIC_THRESHOLD_OFFSET;
/// Claim/complete base address.
pub const PLIC_CLAIM_BASE: usize = PLIC_BASE + PLIC_CLAIM_OFFSET;

// ============================================================================
// VIRTIO
// ============================================================================

pub const VIRTIO_BASE: usize = 0x1000_1000;
pub const VIRTIO_SIZE: usize = 0x1000;
pub const VIRTIO_COUNT: usize = 8; // 8 virtio devices

// ============================================================================
// DRAM
// ============================================================================

pub const DRAM_BASE: usize = 0x8000_0000;
