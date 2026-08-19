//! The 16550 UART, which is this kernel's console port.
//!
//! Register layout and access come from the `uart_16550` crate. What belongs here is what
//! ties it to this machine: the names the device tree knows the chip by, and the alias its
//! window is reached through.

use mmu::PhysicalAddr;
use uart_16550::MmioSerialPort;

/// The `compatible` strings this driver binds to, and the only place they appear — the tree
/// walk matches nodes against this list rather than a copy of its own.
pub const COMPATIBLE: &[&str] = &["ns16550a", "ns16550"];

/// Adopt the port whose registers begin at `base`, and hand it back ready to write.
///
/// The port arrives configured, because this chip is the firmware's console too: OpenSBI
/// names it `uart8250` in the banner it prints on the way out, and keeps writing there for
/// whatever M-mode has to report. Adopting it is the whole of binding. The line status
/// register says when the transmitter will take a byte, and it says so on a port somebody
/// else configured.
///
/// Reprogramming one already in use costs output. `MmioSerialPort::init` clears the transmit
/// FIFO, discarding whatever firmware queued and has not yet shifted out; raises DLAB, for
/// which window offset 0 is the divisor latch rather than the transmit register, so an
/// M-mode byte lands in the baud rate; installs a divisor of its own, which on hardware that
/// honours one leaves every later byte at a speed the host is not listening at; and enables
/// receive interrupts on a line with no handler.
///
/// Binding the UART the device tree names takes it to be the one firmware prints to. A
/// platform that separates the two states its own divisor.
///
/// Driven through the direct map rather than the physical base: a port kept in a `static`
/// outlives the boot table, and because that map is linear the alias holds under the kernel
/// table too.
///
/// # Safety
/// `base` must begin a 16550's MMIO window, mapped readable and writable, and this must be
/// the only port this kernel builds for it. Firmware's own driver writes the same registers
/// from M-mode, under a lock that does not span the two — see [`crate::console`].
pub unsafe fn bind(base: PhysicalAddr) -> MmioSerialPort {
    let window = crate::memory::direct_map::phys_to_virt(base);
    // SAFETY: forwarded from this function's contract — `window` is the direct-map alias of
    // a mapped 16550 window, and the caller keeps it exclusive among this kernel's users.
    unsafe { MmioSerialPort::new(window.bits()) }
}
