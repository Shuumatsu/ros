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
fn write(fd: usize, base: VirtualAddr, len: usize) -> Answer {
    if fd != STDOUT {
        return Err(Error::BadDescriptor);
    }

    user::read(base, len, |bytes| {
        // A console writes text. Everything up to the first byte that is not part of a UTF-8
        // sequence is written and answered for; the rest has no rendering to reach the wire with.
        let text = bytes.utf8_chunks().next().map_or("", |chunk| chunk.valid());
        // The locked console, because a program's output is a routine event and user mode holds no
        // kernel lock to deadlock against. The line ending is the console's — a serial terminal
        // needs `\r\n` — so a trailing newline is the program asking for the break `println!`
        // already provides.
        println!("{}", text.strip_suffix('\n').unwrap_or(text));
        text.len()
    })
    .ok_or(Error::BadAddress)
}
