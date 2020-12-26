use core::fmt;
use lazy_static::lazy_static;
use spin::Mutex;

use crate::drivers::serial::uart16550::UartDriver;
use crate::memory::layout::UART_BASE_ADDR;

lazy_static! {
    pub static ref UART_DRIVER: Mutex<UartDriver> = Mutex::new({
        let mut driver = UartDriver::new(UART_BASE_ADDR);
        driver.init();
        driver
    });
}

pub struct Stdout;

impl fmt::Write for Stdout {
    fn write_str(&mut self, out: &str) -> fmt::Result {
        for c in out.bytes() {
            UART_DRIVER.lock().put_sync(c);
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
