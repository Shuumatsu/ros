use core::fmt::{self, Write};

use riscv::interrupt;
use spin::Mutex;

use crate::{cpu, sbi::console_putchar};

struct Stdout;

impl Write for Stdout {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            console_putchar(c as usize);
        }
        Ok(())
    }
}

lazy_static! {
    static ref STDOUT: Mutex<Stdout> = Mutex::new(Stdout);
}

pub fn print(args: fmt::Arguments) {
    cpu::without_interrupts(|| {
        STDOUT.lock().write_fmt(args).unwrap();
    })
}

#[macro_export]
macro_rules! print {
    ($fmt: literal $(, $($arg: tt)+)?) => {
        $crate::console::print(format_args!($fmt $(, $($arg)+)?));
    }
}

#[macro_export]
macro_rules! println {
    ($fmt: literal $(, $($arg: tt)+)?) => {
        $crate::console::print(format_args!(concat!($fmt, "\n") $(, $($arg)+)?));
    }
}
