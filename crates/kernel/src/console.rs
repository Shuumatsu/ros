use core::fmt::{self, Write};
use spin::{Lazy, Mutex};
use uart_16550::MmioSerialPort;

use crate::arch::riscv64::hart_id;

pub static UART: Lazy<Mutex<MmioSerialPort>> = Lazy::new(|| {
    // Base comes from the device tree (`device_tree::discover` runs before the
    // first print); the earlycon default only applies before discovery.
    let mut serial = unsafe { MmioSerialPort::new(crate::device_tree::uart_base()) };
    serial.init();
    Mutex::new(serial)
});

/// Lock-free stdout for interrupt/panic contexts
struct KernelStdout;

impl fmt::Write for KernelStdout {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let uart = crate::device_tree::uart_base() as *mut u8;
        for c in s.bytes() {
            unsafe { uart.write_volatile(c) };
        }
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
        let mut uart = UART.lock();
        let _ = write!(uart, "[hart {}] ", hart);
        let _ = uart.write_fmt(args);
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
