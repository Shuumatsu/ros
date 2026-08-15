//! Everything between the firmware's jump and the first ordinary Rust.
//!
//! - [`asm`] — `boot_asm!`, which the four below assemble through.
//! - [`image`] — the RISC-V Image header the loader parses, and `_start`.
//! - [`entry`] — the two ISA entry points and the low-to-high transition.
//! - [`primary`] / [`secondary`] — one prologue per kind of hart, since they need
//!   different stacks under different page tables.

#[macro_use]
mod asm;

mod entry;
mod image;
mod primary;
mod secondary;

pub(crate) use entry::secondary_entry_address;
pub(crate) use secondary::SecondaryHandoff;
