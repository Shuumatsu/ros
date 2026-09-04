//! RISC-V Linux-compatible system-call numbers and result encoding.

/// `write(fd, buf, len)`.
pub const WRITE: usize = 64;

/// `exit(status)`.
pub const EXIT: usize = 93;

/// Standard output.
pub const STDOUT: usize = 1;

/// Linux-compatible error numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Error {
    BadDescriptor = 9,
    BadAddress = 14,
    NoSuchCall = 38,
}

pub type Answer = Result<usize, Error>;

/// Encodes success as a non-negative count and failure as negated `errno`.
///
/// Successful values must fit in `isize`.
pub const fn encode(answer: Answer) -> isize {
    match answer {
        Ok(count) => count as isize,
        Err(error) => -(error as isize),
    }
}
