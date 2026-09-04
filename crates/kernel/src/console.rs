//! Kernel console output.
//!
//! Normal output uses an interrupt-masking lock. Emergency output bypasses that lock through
//! SBI. Both paths use SBI until the device-tree UART is available.

use core::fmt::{self, Write};
use uart_16550::MmioSerialPort;

use crate::arch::sbi;
use crate::cpu;
use crate::device_tree;
use crate::drivers::uart16550;
use crate::sync::IrqMutex;

static UART: IrqMutex<Option<MmioSerialPort>> = IrqMutex::new(None);

fn emit(port: &mut Option<MmioSerialPort>, bytes: &[u8]) {
    if port.is_none()
        && let Some(base) = device_tree::uart_base()
    {
        // SAFETY: the DTB matched this driver's compatibility list, and `UART` is the sole owner.
        *port = Some(unsafe { uart16550::bind(base) });
    }
    match port {
        Some(serial) => framed(bytes, |byte| serial.send_raw(byte)),
        None => sbi_write(bytes),
    }
}

/// Converts line feeds to CRLF and otherwise sends bytes unchanged.
fn framed(bytes: &[u8], mut put: impl FnMut(u8)) {
    for &byte in bytes {
        if byte == b'\n' {
            put(b'\r');
        }
        put(byte);
    }
}

/// Lock-free SBI output that requires no mapped buffer.
fn sbi_write(bytes: &[u8]) { framed(bytes, sbi::console_write_byte) }

struct Uart<'a>(&'a mut Option<MmioSerialPort>);
impl fmt::Write for Uart<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        emit(self.0, s.as_bytes());
        Ok(())
    }
}

struct SbiConsole;
impl fmt::Write for SbiConsole {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        sbi_write(s.as_bytes());
        Ok(())
    }
}

/// Uses `?` before the hart adopts its CPU control block.
fn write_prefix(out: &mut impl Write) {
    match cpu::try_hart_id() {
        Some(hart) => {
            let _ = write!(out, "[hart {hart}] ");
        }
        None => {
            let _ = out.write_str("[hart ?] ");
        }
    }
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    // Keep each prefixed message indivisible across harts.
    UART.with(|port| {
        let mut out = Uart(port);
        write_prefix(&mut out);
        let _ = out.write_fmt(args);
    });
}

/// Writes one indivisible byte stream without a prefix or added line ending.
pub fn write_bytes(bytes: &[u8]) { UART.with(|port| emit(port, bytes)) }

#[doc(hidden)]
pub fn _emergency_print(args: fmt::Arguments) {
    write_prefix(&mut SbiConsole);
    let _ = SbiConsole.write_fmt(args);
}

// Interrupt masking makes the locked path safe to use from interrupt handlers.
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => { $crate::console::_print(format_args!($($arg)*)) };
}
#[macro_export]
macro_rules! println {
    () => { $crate::print!("\n") };
    ($($arg:tt)*) => { $crate::print!("{}\n", format_args!($($arg)*)) };
}

// Emergency output avoids lock recursion on fatal paths and may interleave bytewise across harts.
#[macro_export]
macro_rules! emergency_print {
    ($($arg:tt)*) => { $crate::console::_emergency_print(format_args!($($arg)*)) };
}

#[macro_export]
macro_rules! emergency_println {
    () => { $crate::emergency_print!("\n") };
    ($($arg:tt)*) => { $crate::emergency_print!("{}\n", format_args!($($arg)*)) };
}
