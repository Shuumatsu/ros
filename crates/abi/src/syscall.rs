//! System-call numbers and the descriptors they take.
//!
//! The numbers are Linux's for riscv64, borrowed rather than invented so that a program built
//! against a real libc asks this kernel for what it means.

/// `write(fd, buf, len)`, answering with the byte count or a negative error.
pub const WRITE: usize = 64;

/// `exit(status)`, which does not return.
pub const EXIT: usize = 93;

/// The descriptor the kernel writes to its console.
pub const STDOUT: usize = 1;
