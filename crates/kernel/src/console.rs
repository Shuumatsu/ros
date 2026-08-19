//! Kernel output: `print!`/`println!`, and the two sinks behind them.
//!
//! Two paths, because the ordinary one has a prerequisite the fatal one cannot assume.
//! [`_print`] takes a lock and writes to the device-tree UART; [`_emergency_print`] takes
//! nothing and writes a byte at a time through SBI, so it still works when this hart
//! already holds the lock or the UART's own mapping is what broke.
//!
//! No address is hardcoded. Until the device tree yields a UART base, both paths are the
//! SBI console, which is what lets the earliest code print at all.
//!
//! Firmware's console is the same chip, written from M-mode under a lock this side does not
//! share, so an M-mode trap report interleaves with these writes byte by byte. The overlap
//! costs ordering rather than content: [`uart16550::bind`] adopts the port instead of
//! reprogramming it, and every line carries its own `[hart N]`, so a shuffled log still reads.

use core::fmt::{self, Write};
use uart_16550::MmioSerialPort;

use crate::arch::sbi;
use crate::cpu;
use crate::device_tree;
use crate::drivers::uart16550;
use crate::sync::IrqMutex;

/// The primary MMIO UART, bound to the device-tree base on the first print after the DTB
/// is parsed; until then output falls back to the SBI console, so no address is hardcoded.
///
/// An [`IrqMutex`]: a handler printing on a hart already inside this lock would spin
/// against itself forever.
static UART: IrqMutex<Option<MmioSerialPort>> = IrqMutex::new(None);

/// Write `bytes` to the DTB-discovered MMIO UART if we have it, else the SBI console.
fn emit(port: &mut Option<MmioSerialPort>, bytes: &[u8]) {
    if port.is_none()
        && let Some(base) = device_tree::uart_base()
    {
        // SAFETY: `base` is the window of a node the tree matched against this driver's own
        // `compatible` list, mapped R+W, and this is the only port this kernel builds for it —
        // the `UART` mutex keeps that exclusive.
        *port = Some(unsafe { uart16550::bind(base) });
    }
    match port {
        Some(serial) => framed(bytes, |byte| serial.send_raw(byte)),
        None => sbi_write(bytes),
    }
}

/// Put `bytes` on a serial line with a carriage return before every line feed.
///
/// The device's requirement, and the only place it is stated: a terminal moves down a line on LF and
/// returns to the first column on CR, so a lone LF leaves the next line indented by the length of
/// this one. Every caller writes `\n`.
///
/// Raw sends, so a byte goes out as itself. `MmioSerialPort::send` reads 0x08 and 0x7f as a request
/// to erase a character and expands each into three bytes, which is a terminal's convention rather
/// than this line's.
fn framed(bytes: &[u8], mut put: impl FnMut(u8)) {
    for &byte in bytes {
        if byte == b'\n' {
            put(b'\r');
        }
        put(byte);
    }
}

/// Lock-free write via the SBI console — needs no address, so it works when the UART's own
/// mapping is what broke.
///
/// A byte at a time, because the batched `console_write` takes a physical address and this
/// path runs when producing one is the problem. Errors go nowhere: there is nowhere left.
fn sbi_write(bytes: &[u8]) { framed(bytes, sbi::console_write_byte) }

/// `fmt::Write` sink over the locked UART slot (the normal, locked path).
struct Uart<'a>(&'a mut Option<MmioSerialPort>);
impl fmt::Write for Uart<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        emit(self.0, s.as_bytes());
        Ok(())
    }
}

/// Lock-free SBI-console sink, for when the lock cannot be taken.
struct SbiConsole;
impl fmt::Write for SbiConsole {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        sbi_write(s.as_bytes());
        Ok(())
    }
}

/// The `[hart N] ` every line opens with.
///
/// `?` between the architecture entry and `cpu::init_boot`, where `clear_bss` and the
/// layout checks run: demanding a hart id that does not exist yet would panic, and
/// panicking prints, so the console would take down the one thing able to report it.
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
    // One `with`, so the prefix and the message cannot be split by another hart.
    UART.with(|port| {
        let mut out = Uart(port);
        write_prefix(&mut out);
        let _ = out.write_fmt(args);
    });
}

/// Write `bytes` to the console as they are: no `[hart N]`, and no line ending this was not given.
///
/// A running program's output, which shares a device with the kernel's log and shares nothing else
/// with it. A log line is a whole line and carries the hart that decided it; a program's bytes are a
/// stream, and supplying either for them would be the kernel putting words in a process's mouth.
///
/// One `with`, so no other hart can split a program's write.
pub fn write_bytes(bytes: &[u8]) { UART.with(|port| emit(port, bytes)) }

#[doc(hidden)]
pub fn _emergency_print(args: fmt::Arguments) {
    write_prefix(&mut SbiConsole);
    let _ = SbiConsole.write_fmt(args);
}

// The ordinary path: locked, interrupts masked while held, so a handler on this hart
// cannot contend with it. Use this from interrupt handlers too.
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => { $crate::console::_print(format_args!($($arg)*)) };
}
#[macro_export]
macro_rules! println {
    () => { $crate::print!("\n") };
    ($($arg:tt)*) => { $crate::print!("{}\n", format_args!($($arg)*)) };
}

// Lock-free, for the one case the locked path cannot serve: this hart already holds the
// console lock — a fault inside `_print`, or a panic from anywhere — where taking the
// lock would deadlock instead of printing.
//
// NOT for anything else. These writes interleave character-by-character with every other
// hart's, which on a multi-hart machine shreds the output:
//
//     c[hartod 0] [te: 5,r asep_pc: 0xffhaffndlefr]f scausc0e c80o20de2afc:
//
// The name is deliberately alarming, so that reaching for it takes a decision.
#[macro_export]
macro_rules! emergency_print {
    ($($arg:tt)*) => { $crate::console::_emergency_print(format_args!($($arg)*)) };
}

#[macro_export]
macro_rules! emergency_println {
    () => { $crate::emergency_print!("\n") };
    ($($arg:tt)*) => { $crate::emergency_print!("{}\n", format_args!($($arg)*)) };
}
