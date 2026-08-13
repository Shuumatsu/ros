use core::fmt::{self, Write};
use paging::PhysicalAddr;
use uart_16550::MmioSerialPort;

use crate::cpu;
use crate::device_tree;
use crate::sync::IrqMutex;

/// The primary MMIO UART, bound to the device-tree base on the first print after the DTB
/// is parsed; until then output falls back to the SBI console, so no address is hardcoded.
///
/// An [`IrqMutex`]: a handler printing on a hart already inside this lock would spin
/// against itself forever.
static UART: IrqMutex<Option<MmioSerialPort>> = IrqMutex::new(None);

/// Write `s` to the DTB-discovered MMIO UART if we have it, else the SBI console.
fn emit(port: &mut Option<MmioSerialPort>, s: &str) {
    if port.is_none() {
        if let Some(base) = device_tree::uart_base() {
            // Through the direct map, not the raw physical base: this port is cached in
            // a `static` that outlives the boot table, and because the map is linear the
            // alias is canonical and mapped under both tables.
            let uart = crate::memory::phys_to_virt(PhysicalAddr::new(base));
            // SAFETY: `uart` is the direct-map alias of the DTB-reported UART
            // window, mapped R+W, and this is the only `MmioSerialPort` built for
            // it — the `UART` mutex keeps that exclusive.
            let mut serial = unsafe { MmioSerialPort::new(uart.bits()) };
            serial.init();
            *port = Some(serial);
        }
    }
    match port {
        Some(serial) => {
            let _ = serial.write_str(s);
        }
        None => sbi_write(s),
    }
}

/// Lock-free write via the SBI console — needs no address, so it works when the UART's own
/// mapping is what broke.
///
/// A byte at a time, because the batched `console_write` takes a physical address and this
/// path runs when producing one is the problem. Errors go nowhere: there is nowhere left.
fn sbi_write(s: &str) {
    for b in s.bytes() {
        let _ = sbi_rt::console_write_byte(b);
    }
}

/// `fmt::Write` sink over the locked UART slot (the normal, locked path).
struct Uart<'a>(&'a mut Option<MmioSerialPort>);
impl fmt::Write for Uart<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        emit(self.0, s);
        Ok(())
    }
}

/// Lock-free SBI-console sink, for when the lock cannot be taken.
struct SbiConsole;
impl fmt::Write for SbiConsole {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        sbi_write(s);
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
    () => { $crate::print!("\r\n") };
    ($($arg:tt)*) => { $crate::print!("{}\r\n", format_args!($($arg)*)) };
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
// The alarming name is the fix, not the comment: the predecessor `kprintln!` read as a
// drop-in for `println!` and got misused twice.
#[macro_export]
macro_rules! emergency_print {
    ($($arg:tt)*) => { $crate::console::_emergency_print(format_args!($($arg)*)) };
}

#[macro_export]
macro_rules! emergency_println {
    () => { $crate::emergency_print!("\r\n") };
    ($($arg:tt)*) => { $crate::emergency_print!("{}\r\n", format_args!($($arg)*)) };
}
