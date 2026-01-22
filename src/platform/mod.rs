//! Platform-specific constants and configuration.

pub mod qemu_virt;

// Re-export the current platform for convenience.
pub use qemu_virt::*;
