//! User system-call dispatch.

use abi::syscall::{Answer, EXIT, Error, STDOUT, WRITE, encode};
use mmu::VirtualAddr;

use crate::arch::trap::TrapFrame;
use crate::console;
use crate::memory::user;
use crate::process;

/// Serves the call the frame trapped on, and answers it.
pub(crate) fn dispatch(frame: &mut TrapFrame) {
    let (number, [a0, a1, a2]) = frame.syscall();

    let answer = match number {
        WRITE => write(a0, VirtualAddr::new(a1), a2),
        EXIT => process::exit(a0),
        number => {
            println!("[syscall] the process asked for call {number}, which this kernel has not");
            Err(Error::NoSuchCall)
        }
    };

    frame.complete_syscall(encode(answer) as usize);
}

/// `write(fd, buf, len)`: the console, and no other descriptor yet.
///
/// The user-access window remains open for the full synchronous device write.
fn write(fd: usize, base: VirtualAddr, len: usize) -> Answer {
    if fd != STDOUT {
        return Err(Error::BadDescriptor);
    }

    user::read(base, len, |bytes| {
        console::write_bytes(bytes);
        bytes.len()
    })
    .ok_or(Error::BadAddress)
}
