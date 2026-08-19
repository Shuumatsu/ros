//! User processes: what one is loaded from, and what it runs in.
//!
//! [`image`] reads an executable; [`crate::memory::user_table`] turns what it read into an address
//! space. This file holds the one step across them.

pub mod image;

use mmu::VirtualAddr;

use crate::memory::{address_space::AddressSpace, user_table};

/// The one user program this kernel runs, embedded in the image.
///
/// `CARGO_BIN_FILE_HELLO` comes from the artifact dependency in `Cargo.toml`, so cargo builds the
/// program before the kernel and names the file it produced. It moves to the disk once there is a
/// block driver and a filesystem to read it with.
static HELLO: &[u8] = include_bytes!(env!("CARGO_BIN_FILE_HELLO"));

/// Load the embedded program into an address space of its own, and answer with its entry point.
///
/// # Panics
///
/// If the embedded image is not one this kernel can run, which is a build that produced something
/// unexpected rather than a runtime condition.
pub fn load() -> (AddressSpace, VirtualAddr) {
    let image = image::parse(HELLO)
        .unwrap_or_else(|error| panic!("the embedded user image will not load: {error}"));
    let space = user_table::build(&image.segments);
    (space, image.entry)
}
