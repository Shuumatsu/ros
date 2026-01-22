use core::fmt;
use spin::{Lazy, Mutex};
use uart_16550::MmioSerialPort;

use crate::platform::UART0_BASE;

pub static UART: Lazy<Mutex<MmioSerialPort>> = Lazy::new(|| {
    let mut serial = unsafe { MmioSerialPort::new(UART0_BASE) };
    serial.init();
    Mutex::new(serial)
});

pub struct Stdout;

impl fmt::Write for Stdout {
    fn write_str(&mut self, out: &str) -> fmt::Result {
        for c in out.bytes() {
            UART.lock().send(c);
        }
        Ok(())
    }
}

pub static CONSOLE: Mutex<Stdout> = Mutex::new(Stdout);

#[macro_export]
macro_rules! print {
    ($($args: tt)+) => {{
        use crate::console::CONSOLE;
        use core::fmt::Write;

        CONSOLE.lock().write_fmt(format_args!($($args)*)).unwrap();
    }};
}
#[macro_export]
macro_rules! println {
    () => ({ print!("\r\n") });
    ($fmt: expr) => ({
        print!(concat!($fmt, "\r\n"))
    });
    ($fmt: expr, $($args: tt)+) => ({
        print!(concat!($fmt, "\r\n"), $($args)+)
    });
}

#[macro_export]
macro_rules! kprint {
    ($($args: tt)+) => {{
        use crate::console::Stdout;
        use core::fmt::Write;

        Stdout.write_fmt(format_args!($($args)*)).unwrap();
    }};
}
#[macro_export]
macro_rules! kprintln {
    () => ({ kprint!("\r\n") });

    ($fmt: expr) => ({ kprint!(concat!($fmt, "\r\n")) });

    ($fmt: expr, $($args: tt)+) => ({ kprint!(concat!($fmt, "\r\n"), $($args)+) });
}
