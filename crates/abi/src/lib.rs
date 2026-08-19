//! What the kernel and the user programs must agree on.
//!
//! They are separate binaries that never link together, so nothing but this crate keeps them
//! agreed about what an `ecall` means. Every such fact is written here exactly once, and both
//! sides read it rather than restating it — the arrangement `blockdev` gives the filesystem and
//! its backends, applied to the other boundary this kernel has.
//!
//! [`syscall`] is the contract itself, plain numbers that build anywhere, which is what lets host
//! tools and tests see them. [`call`] is the *calling* side and exists only for the instruction
//! set that has the instruction.

#![no_std]

pub mod syscall;

#[cfg(target_arch = "riscv64")]
pub mod call;
