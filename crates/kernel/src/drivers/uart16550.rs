//! 16550 console UART support.

use mmu::PhysicalAddr;
use uart_16550::MmioSerialPort;

pub const COMPATIBLE: &[&str] = &["ns16550a", "ns16550"];

/// Adopts the firmware-configured port through its permanent direct-map alias.
///
/// # Safety
///
/// `base` must identify a mapped 16550 MMIO window, and the returned port must be the
/// kernel's only accessor to it.
pub unsafe fn bind(base: PhysicalAddr) -> MmioSerialPort {
    let window = crate::memory::direct_map::phys_to_virt(base);
    // SAFETY: the direct-map alias preserves the caller's MMIO and exclusivity guarantees.
    unsafe { MmioSerialPort::new(window.bits()) }
}
