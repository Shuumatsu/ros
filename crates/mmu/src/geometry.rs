//! Paging geometry shared by RV64 translation schemes.
//!
//! ```text
//! Virtual address (12 + 9 bits per level, sign-extended to 64):
//!   | ... | VPN[1] 29:21 | VPN[0] 20:12 | offset 11:0 |
//! Physical address (56 bits):
//!   | PPN 55:12 | offset 11:0 |
//! Page-table entry (identical in every scheme):
//!   | PPN 53:10 | RSW 9:8 | DAGUXWRV 7:0 |
//! ```

pub const PAGE_OFFSET_BITS: usize = 12;
pub const PAGE_SIZE: usize = 1 << PAGE_OFFSET_BITS;

pub const VPN_BITS: usize = 9;

/// Maximum level count representable in an RV64 virtual address.
pub const MAX_LEVELS: usize = (usize::BITS as usize - PAGE_OFFSET_BITS) / VPN_BITS;

pub const ENTRY_SIZE: usize = core::mem::size_of::<u64>();
pub const ENTRIES_PER_PAGE: usize = 1 << VPN_BITS;

/// Root-table slots in one canonical half of the address space.
///
/// Slots `0..256` select the low half and `256..512` the high half.
pub const ROOT_ENTRIES_PER_HALF: usize = ENTRIES_PER_PAGE / 2;

/// Width of the PTE physical-page-number field.
pub const PPN_BITS: usize = 44;

/// Number of bytes mapped by a leaf entry installed at `level`
/// (4 KiB at level 0, 2 MiB at level 1, 1 GiB at level 2, and so on upward).
#[inline]
pub const fn page_size_at(level: usize) -> usize {
    debug_assert!(level < MAX_LEVELS, "level out of range");
    1 << (PAGE_OFFSET_BITS + VPN_BITS * level)
}

pub const SUPERPAGE: usize = page_size_at(1);
pub const GIGAPAGE: usize = page_size_at(2);

const_assert_eq!(ENTRIES_PER_PAGE, 512);
const_assert_eq!(ENTRIES_PER_PAGE * ENTRY_SIZE, PAGE_SIZE);
const_assert_eq!(page_size_at(0), PAGE_SIZE);
const_assert_eq!(PAGE_SIZE, 4096);
const_assert_eq!(SUPERPAGE, 2 * 1024 * 1024);
const_assert_eq!(GIGAPAGE, 1024 * 1024 * 1024);
const_assert_eq!(PAGE_OFFSET_BITS + VPN_BITS * MAX_LEVELS, 57);
const_assert_eq!(MAX_LEVELS, 5);
