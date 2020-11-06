use core::fmt;

pub fn _print(args: fmt::Arguments) {
    use crate::uart::UART_DRIVER;
    use core::fmt::Write;

    UART_DRIVER.lock().write_fmt(args).unwrap();
}
#[macro_export]
macro_rules! print {
    ($($args:tt)+) => ({
        crate::macros::_print(format_args!($($args)*));
    });
}
#[macro_export]
macro_rules! println   {
    () => ({ print!("\r\n") });
    ($fmt:expr) => ({
        print!(concat!($fmt, "\r\n"))
    });
    ($fmt:expr, $($args:tt)+) => ({
        print!(concat!($fmt, "\r\n"), $($args)+)
    });
}
