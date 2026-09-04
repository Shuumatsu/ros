//! The system-call ABI shared by the kernel and user programs.
//!
//! `call` is available only on RISC-V 64-bit targets.

#![no_std]

pub mod syscall;

#[cfg(target_arch = "riscv64")]
pub mod call;
