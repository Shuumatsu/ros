//! What an `ecall` from user mode asks for, and what the kernel answers.
//!
//! The numbers and the error values are [`abi::syscall`]'s and are not restated here: the contract
//! is one crate both sides read. What this file owns is the table — which number reaches which
//! kernel operation — and the two things every call needs whatever it is: the instruction after the
//! `ecall` to return to, and the single register the answer travels in.
//!
//! Arguments come out of the trap frame and the answer goes back into it, which is what makes a
//! call a normal return from a trap. No user pointer is dereferenced here:
//! [`crate::memory::user`] owns what a user address is and what it takes to read one.

use abi::syscall::{Answer, EXIT, Error, STDOUT, WRITE, encode};
use mmu::VirtualAddr;

use crate::arch::trap::TrapFrame;
use crate::console;
use crate::memory::user;
use crate::process;

/// Bytes to step `sepc` past the instruction that trapped.
///
/// `ecall` is a 32-bit instruction and the C extension defines no compressed form of it, so the
/// instruction after the one that trapped is always four bytes on.
const ECALL_BYTES: usize = 4;

/// Answer the call the frame carries.
///
/// `sepc` is advanced first and unconditionally: a call that returns must not run its `ecall` a
/// second time, and a call that does not return has no `sepc` left to care about.
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
/// The bytes go out as they are and every one of them is answered for. What they mean is the
/// program's business: this kernel neither reads them as text nor supplies a line ending, so a
/// program that writes half a line has written half a line.
///
/// The read window spans the write to the device, which is as long as the device takes.
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
