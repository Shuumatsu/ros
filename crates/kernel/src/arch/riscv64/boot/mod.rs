//! Boot assembly and hart startup.

#[macro_use]
mod asm;

mod entry;
mod image;
mod primary;
mod secondary;

pub(crate) use secondary::{SecondaryHandoff, StartError, entry_address, start_cpu};
