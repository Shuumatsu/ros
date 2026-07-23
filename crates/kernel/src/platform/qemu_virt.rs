//! Platform defaults and register offsets for the QEMU RISC-V virt machine.
//!
//! Device *base addresses* are no longer hardcoded here — they are discovered
//! from the device tree at boot (see `device_tree.rs`). What remains is:
//!
//!  * **Earlycon defaults** (`UART0_*`): used only in the pre-discovery window
//!    so a very early panic is still visible; superseded by the DTB values.
//!  * **The DRAM anchor** (`DRAM_BASE`): identifies which `/memory` region backs
//!    the kernel.
//!  * **Register offsets** (PLIC/CLINT): fixed by the controller spec, not by
//!    the platform memory map, so they are the same regardless of base.
//!
//! Reference: https://github.com/qemu/qemu/blob/master/hw/riscv/virt.c
//!
//! Memory map (for reference; the live values come from the DTB):
//! ```text
//! [VIRT_CLINT]     = { 0x2000000,  0x10000 }
//! [VIRT_PLIC]      = { 0xc000000,  0x600000 }
//! [VIRT_UART0]     = { 0x10000000, 0x100 }
//! [VIRT_DRAM]      = { 0x80000000, ... }
//! ```

// ============================================================================
// UART (NS16550A) — earlycon defaults only
// ============================================================================

/// Earlycon UART base: the address used to print before the device tree has
/// been parsed. Once `device_tree::discover` runs, the DTB value takes over.
pub const UART0_BASE: usize = 0x1000_0000;
pub const UART0_SIZE: usize = 0x100;
pub const UART0_IRQ: usize = 10;

// ============================================================================
// CLINT (Core Local Interruptor) — register offsets from the discovered base
// ============================================================================

/// Offset of hart 0's `mtimecmp` register within the CLINT (add `8 * hartid`).
pub const CLINT_MTIMECMP_OFFSET: usize = 0x4000;
/// Offset of the `mtime` register within the CLINT.
pub const CLINT_MTIME_OFFSET: usize = 0xBFF8;

// ============================================================================
// PLIC (Platform-Level Interrupt Controller) — register offsets
// ============================================================================
//
// All offsets are relative to the PLIC base discovered from the device tree.
//   base + 0x000000: source priorities
//   base + 0x001000: pending bits
//   base + 0x002000: enable bits (context 0)
//   base + 0x200000: priority threshold (context 0)
//   base + 0x200004: claim/complete (context 0)

/// Interrupt priority for each source.
pub const PLIC_PRIORITY_OFFSET: usize = 0x0;
/// Interrupt pending status.
pub const PLIC_PENDING_OFFSET: usize = 0x1000;
/// Interrupt enable bits per context.
pub const PLIC_ENABLE_OFFSET: usize = 0x2000;
/// Priority threshold per context.
pub const PLIC_THRESHOLD_OFFSET: usize = 0x20_0000;
/// Claim/complete register per context.
pub const PLIC_CLAIM_OFFSET: usize = 0x20_0004;

// ============================================================================
// DRAM
// ============================================================================

/// The base of main memory. Used to pick which `/memory` region in the device
/// tree backs the kernel; the region's actual size comes from the DTB.
pub const DRAM_BASE: usize = 0x8000_0000;
