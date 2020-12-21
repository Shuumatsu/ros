use core::fmt;

pub fn _print(with_cpu: bool, args: fmt::Arguments) {
    use crate::arch::riscv64 as arch;
    use crate::uart::UART_DRIVER;
    use core::fmt::Write;

    let mut driver = UART_DRIVER.lock();

    // if with_cpu {
    //     driver
    //         .write_fmt(format_args!("[cpu {}, sp: {:#x}] ", arch::hart_id(), arch::stack_pointer()))
    //         .unwrap();
    // }
    driver.write_fmt(args).unwrap();
}
#[macro_export]
macro_rules! print {
    ($($args:tt)+) => ({
        crate::log::_print(false, format_args!($($args)*));
    });
}
#[macro_export]
macro_rules! println {
    () => ({ print!("\r\n") });
    ($fmt:expr) => ({
        print!(concat!($fmt, "\r\n"))
    });
    ($fmt:expr, $($args:tt)+) => ({
        print!(concat!($fmt, "\r\n"), $($args)+)
    });
}
#[macro_export]
macro_rules! kprint {
    ($($args:tt)+) => ({
        crate::log::_print(true, format_args!($($args)*));
    });
}
#[macro_export]
macro_rules! kprintln {
    () => ({ kprint!("\r\n") });
    ($fmt:expr) => ({
        kprint!(concat!($fmt, "\r\n"))
    });
    ($fmt:expr, $($args:tt)+) => ({
        kprint!(concat!($fmt, "\r\n"), $($args)+)
    });
}
