//! The paging geometry every RV64 translation scheme shares.
//!
//! Sv39, Sv48 and Sv57 differ in exactly one thing: how many levels a walk descends.
//! Everything here is common to all three, so it is stated once, at the crate root, and
//! [`Scheme`](crate::Scheme) carries the difference.
//!
//! ```text
//! Virtual address (12 + 9 bits per level, sign-extended to 64):
//!   | ... | VPN[1] 29:21 | VPN[0] 20:12 | offset 11:0 |
//! Physical address (56 bits):
//!   | PPN 55:12 | offset 11:0 |
//! Page-table entry (identical in every scheme):
//!   | PPN 53:10 | RSW 9:8 | DAGUXWRV 7:0 |
//! ```

use crate::utils::{GIGABYTE, KILOBYTE, MEGABYTE};

/// Bits of byte offset within a base page.
pub const PAGE_OFFSET_BITS: usize = 12;
/// Size of a base page in bytes.
pub const PAGE_SIZE: usize = 4 * KILOBYTE;

/// Virtual-page-number index bits consumed per level.
pub const VPN_BITS: usize = 9;

/// Levels the deepest scheme can walk.
///
/// A ceiling rather than a census: each level consumes [`VPN_BITS`] of virtual address
/// above the page offset, so a sixth would need 66 bits and an RV64 address has 64. Sv57
/// is therefore as deep as the family goes, which [`Scheme`](crate::Scheme) asserts.
///
/// This bounds [`page_size_at`], the one accessor that names a level without naming a
/// scheme. A walk has a scheme, and [`vpn`](crate::vpn) holds it to that scheme's own
/// count.
pub const MAX_LEVELS: usize = (usize::BITS as usize - PAGE_OFFSET_BITS) / VPN_BITS;

/// Size of one page-table entry in bytes.
pub const ENTRY_SIZE: usize = core::mem::size_of::<u64>();
/// Number of entries in a single page table (fills exactly one page).
pub const ENTRIES_PER_PAGE: usize = 1 << VPN_BITS;

/// Root-table slots in one canonical half of the address space.
///
/// Every scheme splits the root evenly — slots `0..256` are the low half, `256..512` the
/// high half, and every address between the two is non-canonical. A kernel that puts
/// itself in the high half and users in the low one is dividing the root along exactly
/// this line.
pub const ROOT_ENTRIES_PER_HALF: usize = ENTRIES_PER_PAGE / 2;

/// Total width of a physical page number.
///
/// 44 bits in every scheme: the PTE reserves the same field regardless of how many levels
/// index into it, which is why [`crate::Entry`] needs no scheme of its own.
pub const PPN_BITS: usize = 44;

/// Number of bytes mapped by a leaf entry installed at `level`
/// (4 KiB at level 0, 2 MiB at level 1, 1 GiB at level 2, and so on upward).
#[inline]
pub const fn page_size_at(level: usize) -> usize {
    debug_assert!(level < MAX_LEVELS, "level out of range");
    1 << (PAGE_OFFSET_BITS + VPN_BITS * level)
}

/// Bytes mapped by one level-1 leaf.
pub const SUPERPAGE: usize = page_size_at(1);
/// Bytes mapped by one level-2 leaf. The root leaf of Sv39, an intermediate one above it.
pub const GIGAPAGE: usize = page_size_at(2);

const_assert_eq!(ENTRIES_PER_PAGE, 512);
const_assert_eq!(ENTRIES_PER_PAGE * ENTRY_SIZE, PAGE_SIZE);
const_assert_eq!(page_size_at(0), PAGE_SIZE);
const_assert_eq!(SUPERPAGE, 2 * MEGABYTE);
const_assert_eq!(GIGAPAGE, GIGABYTE);
// The ceiling is Sv57's width exactly: five VPN fields over a page offset.
const_assert_eq!(PAGE_OFFSET_BITS + VPN_BITS * MAX_LEVELS, 57);
const_assert_eq!(MAX_LEVELS, 5);
