//! RISC-V kernel entry crate.

#![no_std]
#![no_main]
#![feature(alloc_error_handler)]
#![feature(abi_custom)]

extern crate alloc;

mod arch;
mod cpu;
#[macro_use]
mod console;
mod device_tree;
mod drivers;
mod memory;
mod process;
mod start;
mod sync;
mod syscall;
mod time;
mod trap;
mod utils;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    if let Some(p) = info.location() {
        emergency_println!("Aborting: file {}:{}: \n\t{}", p.file(), p.line(), info.message());
    } else {
        emergency_println!("Aborting: no information available.");
    }
    abort();
}

/// Compiler-referenced fatal-path symbol; the kernel does not unwind.
#[unsafe(no_mangle)]
extern "C" fn abort() -> ! {
    emergency_println!("enter abort()");
    arch::wait_forever()
}
