//! User system-call dispatch.

use abi::syscall::{Answer, EXIT, Error, STDOUT, WRITE, encode};
use mmu::VirtualAddr;

use crate::arch::trap::TrapFrame;
use crate::console;
use crate::memory::user;
use crate::process;

/// `ecall` is always four bytes; the compressed extension defines no shorter encoding.
const ECALL_BYTES: usize = 4;

/// Dispatches the frame's call after advancing `sepc` past `ecall`.
pub(crate) fn dispatch(frame: &mut TrapFrame) {
    frame.sepc += ECALL_BYTES;

    let answer = match frame.a7 {
        WRITE => write(frame.a0, VirtualAddr::new(frame.a1), frame.a2),
        EXIT => process::exit(frame.a0),
        number => {
            println!("[syscall] the process asked for call {number}, which this kernel has not");
            Err(Error::NoSuchCall)
        }
    };

    frame.a0 = encode(answer) as usize;
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
