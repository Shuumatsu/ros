use core::fmt::{self, Write};
use spin::Mutex;
use uart_16550::MmioSerialPort;

use crate::arch::riscv64::{hart_id, sbi};
use crate::device_tree;

/// The primary MMIO UART, bound to the device-tree base the first time we print
/// after the DTB is parsed. `None` until then, when output falls back to the SBI
/// console — so no UART address is ever hardcoded.
static UART: Mutex<Option<MmioSerialPort>> = Mutex::new(None);

/// Write `s` to the DTB-discovered MMIO UART if we have it, else the SBI console.
fn emit(port: &mut Option<MmioSerialPort>, s: &str) {
    if port.is_none() {
        if let Some(base) = device_tree::uart_base() {
            let mut serial = unsafe { MmioSerialPort::new(base) };
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

/// Lock-free write via the SBI console — needs no address, safe in panic/IRQ.
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

/// Lock-free SBI-console sink for interrupt/panic contexts.
struct KernelStdout;
impl fmt::Write for KernelStdout {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        sbi_write(s);
        Ok(())
    }
}

/// Disable supervisor interrupts, returns whether they were enabled
#[inline]
fn disable_interrupts() -> bool {
    use riscv::register::sstatus;
    let was_enabled = sstatus::read().sie();
    if was_enabled {
        unsafe {
            sstatus::clear_sie();
        }
    }
    was_enabled
}

/// Restore supervisor interrupts if they were previously enabled
#[inline]
fn restore_interrupts(was_enabled: bool) {
    if was_enabled {
        unsafe {
            riscv::register::sstatus::set_sie();
        }
    }
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    let hart = hart_id();

    // Disable interrupts to prevent deadlock while holding UART lock
    let was_enabled = disable_interrupts();

    {
        let mut port = UART.lock();
        let mut out = Uart(&mut port);
        let _ = write!(out, "[hart {}] ", hart);
        let _ = out.write_fmt(args);
    }

    restore_interrupts(was_enabled);
}

/// For interrupt/panic contexts - writes directly without lock
#[doc(hidden)]
pub fn _kprint(args: fmt::Arguments) {
    let hart = hart_id();
    let _ = write!(KernelStdout, "[hart {}] ", hart);
    let _ = KernelStdout.write_fmt(args);
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

// kprint!/kprintln! - for when already in interrupt/panic:
// 1. Write directly to UART register (no lock)
// 2. Print "[hart N] message"
// Used only inside interrupt handlers or panic where interrupts are already disabled.
#[macro_export]
macro_rules! kprint {
    ($($arg:tt)*) => { $crate::console::_kprint(format_args!($($arg)*)) };
}

#[macro_export]
macro_rules! kprintln {
    () => { $crate::kprint!("\r\n") };
    ($($arg:tt)*) => { $crate::kprint!("{}\r\n", format_args!($($arg)*)) };
}
