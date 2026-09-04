//! Architecture-specific kernel support.
//!
//! The rest of the kernel names this module, never the implementation below it. Only one
//! implementation exists: `crates/kernel/Cargo.toml` forces a `riscv64` target.

mod riscv64;

pub use riscv64::*;
