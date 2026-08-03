use core::fmt::{self, Write};
use spin::Mutex;
use uart_16550::MmioSerialPort;

use crate::arch::riscv64::{hart_id, interrupts, sbi};
use crate::device_tree;

/// The primary MMIO UART, bound to the device-tree base the first time we print
/// after the DTB is parsed. `None` until then, when output falls back to the SBI
/// console — so no UART address is ever hardcoded.
static UART: Mutex<Option<MmioSerialPort>> = Mutex::new(None);

/// Write `s` to the DTB-discovered MMIO UART if we have it, else the SBI console.
fn emit(port: &mut Option<MmioSerialPort>, s: &str) {
    if port.is_none() {
        if let Some(base) = device_tree::uart_base() {
            // The device tree reports a *physical* base; reach it through the
            // kernel's direct map. Not the raw address: that is only a valid
            // pointer while a boot identity mapping happens to exist, and this port
            // is cached in a `static` that outlives the boot table.
            //
            // Valid under both tables, which is the point — the direct map is
            // linear, so `phys_to_virt` of a device address is a canonical Sv39
            // address that `boot.S` and `kernel_table` both map. Under the old
            // RAM-base-skewed offset it would not even have been canonical.
            let uart = crate::memory::phys_to_virt(base);
            // SAFETY: `uart` is the direct-map alias of the DTB-reported UART
            // window, mapped R+W, and this is the only `MmioSerialPort` built for
            // it — the `UART` mutex keeps that exclusive.
            let mut serial = unsafe { MmioSerialPort::new(uart) };
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

/// Lock-free write via the SBI console — needs no address, so it works when the
/// UART's own mapping is the thing that broke.
fn sbi_write(s: &str) {
    for b in s.bytes() {
        sbi::console_putchar(b as usize);
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

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    let hart = hart_id();

    // Masked across the whole locked section so an interrupt handler on THIS hart
    // cannot arrive and try to take a lock this hart already holds.
    interrupts::without(|| {
        let mut port = UART.lock();
        let mut out = Uart(&mut port);
        let _ = write!(out, "[hart {}] ", hart);
        let _ = out.write_fmt(args);
    });
}

#[doc(hidden)]
pub fn _emergency_print(args: fmt::Arguments) {
    let hart = hart_id();
    let _ = write!(SbiConsole, "[hart {}] ", hart);
    let _ = SbiConsole.write_fmt(args);
}

// print!/println! - interrupt-safe, locked:
// 1. Disable supervisor interrupts
// 2. Acquire UART lock
// 3. Print "[hart N] message"
// 4. Release lock
// 5. Restore interrupt state
// No deadlock possible - interrupts can't fire while holding the lock.
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => { $crate::console::_print(format_args!($($arg)*)) };
}
#[macro_export]
macro_rules! println {
    () => { $crate::print!("\r\n") };
    ($($arg:tt)*) => { $crate::print!("{}\r\n", format_args!($($arg)*)) };
}

// emergency_print!/emergency_println! - lock-free, for the one situation the locked
// path cannot serve: THIS hart already holds the console lock. That happens when a
// fault is taken inside `_print` itself, and when panicking from anywhere. Taking the
// lock there would deadlock instead of printing, which is exactly when the message
// matters most.
//
// NOT for anything else. Lock-free writes interleave character-by-character with
// every other hart's output; with one hart that is invisible, with several it shreds
// the console into unreadable garbage:
//
//     c[hartod 0] [te: 5,r asep_pc: 0xffhaffndlefr]f scausc0e c80o20de2afc:
//
// The name is deliberately long and alarming. Its predecessor was `kprintln!`, one
// letter from `println!` and reading like a drop-in for it, and that name alone was
// enough for the mistake to be made twice independently — in the trap handler and in
// `kmain` — each time looking perfectly reasonable in review. Renaming it is the fix;
// the comment is only the explanation.
//
// Ordinary logging, including from interrupt handlers, uses `println!`: `_print`
// masks interrupts while it holds the lock, so an interrupt cannot arrive to
// contend with it on this hart.
#[macro_export]
macro_rules! emergency_print {
    ($($arg:tt)*) => { $crate::console::_emergency_print(format_args!($($arg)*)) };
}

#[macro_export]
macro_rules! emergency_println {
    () => { $crate::emergency_print!("\r\n") };
    ($($arg:tt)*) => { $crate::emergency_print!("{}\r\n", format_args!($($arg)*)) };
}
