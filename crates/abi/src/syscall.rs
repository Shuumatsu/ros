//! System-call numbers, the descriptors they take, and how a call that fails says so.
//!
//! The numbers are Linux's for riscv64, borrowed rather than invented so that a program built
//! against a real libc asks this kernel for what it means.

/// `write(fd, buf, len)`, answering with the byte count or a negative error.
pub const WRITE: usize = 64;

/// `exit(status)`, which does not return.
pub const EXIT: usize = 93;

/// The descriptor the kernel writes to its console.
pub const STDOUT: usize = 1;

/// Why a call could not be answered. Linux's `errno` values for the same conditions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Error {
    /// The descriptor is not one this kernel writes to. `EBADF`.
    BadDescriptor = 9,
    /// A pointer argument does not describe memory the process owns. `EFAULT`.
    BadAddress = 14,
    /// No call has this number. `ENOSYS`.
    NoSuchCall = 38,
}

/// What a call answers: a count, or why it could not be answered.
pub type Answer = Result<usize, Error>;

/// An [`Answer`] as the single register a call returns in.
///
/// A count is non-negative and an error is its `errno` negated, so the sign is the discriminant
/// and one register carries both. Every call this contract has answers with a count that fits an
/// `isize`, since a call can never write more bytes than a process can address.
pub const fn encode(answer: Answer) -> isize {
    match answer {
        Ok(count) => count as isize,
        Err(error) => -(error as isize),
    }
}
