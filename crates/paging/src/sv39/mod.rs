//! Sv39 paging mode.
//!
//! Sv39 maps a 39-bit virtual address space through three levels of page
//! tables. This module owns the *geometry* of that layout — the level count,
//! the field widths and the page sizes — so that [`addr`], [`entry`] and
//! [`table`] all derive their shifts and masks from one place.
//!
//! ```text
//! Virtual address (39 significant bits, sign-extended to 64):
//!   | VPN[2] 38:30 | VPN[1] 29:21 | VPN[0] 20:12 | offset 11:0 |
//! Physical address (56 bits):
//!   | PPN[2] 55:30 | PPN[1] 29:21 | PPN[0] 20:12 | offset 11:0 |
//! Page-table entry:
//!   | PPN[2] 53:28 | PPN[1] 27:19 | PPN[0] 18:10 | RSW | DAGUXWRV |
//! ```

pub mod access;
pub mod addr;
pub mod entry;
pub mod frames;
pub mod mapper;
pub mod table;

pub use access::{Identity, LinearOffset, PhysAccess};
pub use addr::{MemoryAddr, PhysicalAddr, VirtualAddr};
pub use entry::{Entry, PteFlags};
pub use frames::FrameSource;
pub use mapper::{MapError, Mapper, Unmapped};
pub use table::Table;

use crate::utils::{GIGABYTE, KILOBYTE, MEGABYTE};

/// Bits of byte offset within a base page.
pub const PAGE_OFFSET_BITS: usize = 12;
/// Size of a base page in bytes.
pub const PAGE_SIZE: usize = 4 * KILOBYTE;

/// Virtual-page-number index bits consumed per level.
pub const VPN_BITS: usize = 9;
/// Number of page-table levels walked (Sv39 → 3).
pub const LEVELS: usize = 3;
/// Index of the root level, where every table walk begins.
pub const ROOT_LEVEL: usize = LEVELS - 1;

/// Size of one page-table entry in bytes.
pub const ENTRY_SIZE: usize = core::mem::size_of::<u64>();
/// Number of entries in a single page table (fills exactly one page).
pub const ENTRIES_PER_PAGE: usize = 1 << VPN_BITS;

/// Root-table slots in one canonical half of the address space.
///
/// Sv39 splits the root evenly — slots `0..256` are the low half, `256..512` the
/// high half, and every address between the two is non-canonical. A kernel that
/// puts itself in the high half and users in the low one is dividing the root
/// along exactly this line.
pub const ROOT_ENTRIES_PER_HALF: usize = ENTRIES_PER_PAGE / 2;

/// Total width of a physical page number (PPN\[2:0\]).
pub const PPN_BITS: usize = 44;
/// Width of each `PPN[i]` field. `PPN[2]` is wider to reach the 56-bit
/// physical address space; `VPN[i]` fields are always [`VPN_BITS`] wide.
pub const PPN_FIELD_BITS: [usize; LEVELS] = [9, 9, 26];

/// Number of bytes mapped by a leaf entry installed at `level`
/// (4 KiB at level 0, 2 MiB at level 1, 1 GiB at level 2).
#[inline]
pub const fn page_size_at(level: usize) -> usize {
    debug_assert!(level < LEVELS, "level out of range");
    1 << (PAGE_OFFSET_BITS + VPN_BITS * level)
}

/// Bytes mapped by one middle-level leaf.
pub const SUPERPAGE: usize = page_size_at(1);
/// Bytes mapped by one root-level leaf.
pub const GIGAPAGE: usize = page_size_at(ROOT_LEVEL);

const_assert_eq!(ENTRIES_PER_PAGE, 512);
const_assert_eq!(ENTRIES_PER_PAGE * ENTRY_SIZE, PAGE_SIZE);
const_assert_eq!(page_size_at(0), PAGE_SIZE);
const_assert_eq!(SUPERPAGE, 2 * MEGABYTE);
const_assert_eq!(GIGAPAGE, GIGABYTE);
// The 39 virtual bits are exactly the offset plus one VPN field per level.
const_assert_eq!(PAGE_OFFSET_BITS + VPN_BITS * LEVELS, 39);
