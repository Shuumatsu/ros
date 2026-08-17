//! Everything between the firmware's jump and the first ordinary Rust.
//!
//! - [`asm`] — `boot_fn!`, which the four modules below are defined through.
//! - [`image`] — the RISC-V Image header the loader parses, and `_start`.
//! - [`entry`] — the two ISA entry points, the low-to-high transition, and the vector a
//!   stopped hart parks on.
//! - [`primary`] / [`secondary`] — one prologue per kind of hart, since they need
//!   different stacks under different page tables.

#[macro_use]
mod asm;

mod entry;
mod image;
mod primary;
mod secondary;

pub(crate) use secondary::{SecondaryHandoff, StartError, entry_address, start_cpu};
