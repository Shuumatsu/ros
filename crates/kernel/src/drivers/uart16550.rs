//! The 16550 UART, which is this kernel's console port.
//!
//! Register layout and access come from the `uart_16550` crate. What belongs here is what
//! ties it to this machine: the names the device tree knows the chip by, and the alias its
//! window is reached through.

use paging::PhysicalAddr;
use uart_16550::MmioSerialPort;

/// The `compatible` strings this driver binds to, and the only place they appear — the tree
/// walk matches nodes against this list rather than a copy of its own.
pub const COMPATIBLE: &[&str] = &["ns16550a", "ns16550"];

/// Initialise the port whose registers begin at `base`, and hand it back ready to write.
///
/// Driven through the direct map rather than the physical base: a port kept in a `static`
/// outlives the boot table, and because that map is linear the alias holds under the kernel
/// table too.
///
/// # Safety
/// `base` must begin a 16550's MMIO window, mapped readable and writable, and this must be
/// the only port built for it — a second would race this one on the same registers.
pub unsafe fn bind(base: PhysicalAddr) -> MmioSerialPort {
    let window = crate::memory::direct_map::phys_to_virt(base);
    // SAFETY: forwarded from this function's contract — `window` is the direct-map alias of
    // a mapped 16550 window, and the caller keeps it exclusive.
    let mut port = unsafe { MmioSerialPort::new(window.bits()) };
    port.init();
    port
}
