//! Sv39 paging mode implementation.
//!
//! Sv39 uses a 39-bit virtual address space with three levels of page tables.

pub mod addr;
pub mod entry;
pub mod table;

pub use addr::{MemoryAddr, PhysicalAddr, VirtualAddr};
pub use entry::*;
pub use table::Table;

use crate::utils::KILOBYTE;

pub const PAGE_SIZE: usize = 4 * KILOBYTE;
pub const ENTRY_SIZE: usize = 8;
pub const ENTRIES_PER_PAGE: usize = PAGE_SIZE / ENTRY_SIZE;

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::size_of;

    mod entry_tests {
        use super::*;

        #[test]
        fn test_entry_new_and_bits() {
            let entry = Entry::new(0);
            assert!(!entry.is_valid());
            assert!(!entry.is_read());
            assert!(!entry.is_write());
            assert!(!entry.is_execute());

            let entry = Entry::new(0b11111111); // all flags set
            assert!(entry.is_valid());
            assert!(entry.is_read());
            assert!(entry.is_write());
            assert!(entry.is_execute());
            assert!(entry.is_user());
            assert!(entry.is_global());
            assert!(entry.is_access());
            assert!(entry.is_dirty());
        }

        #[test]
        fn test_entry_flag_constants() {
            // Verify flag bit positions match RISC-V spec
            assert_eq!(VALID, 1 << 0);
            assert_eq!(READ, 1 << 1);
            assert_eq!(WRITE, 1 << 2);
            assert_eq!(EXECUTE, 1 << 3);
            assert_eq!(USER, 1 << 4);
            assert_eq!(GLOBAL, 1 << 5);
            assert_eq!(ACCESS, 1 << 6);
            assert_eq!(DIRTY, 1 << 7);
        }

        #[test]
        fn test_entry_set_clear_flags() {
            let mut entry = Entry::new(0);

            entry.set_valid();
            assert!(entry.is_valid());
            entry.clear_valid();
            assert!(!entry.is_valid());

            entry.set_read();
            assert!(entry.is_read());
            entry.clear_read();
            assert!(!entry.is_read());

            entry.set_write();
            assert!(entry.is_write());
            entry.clear_write();
            assert!(!entry.is_write());

            entry.set_execute();
            assert!(entry.is_execute());
            entry.clear_execute();
            assert!(!entry.is_execute());
        }

        #[test]
        fn test_entry_leaf_vs_branch() {
            // Branch: V=1, R=W=X=0 (points to next level table)
            let branch = Entry::new(VALID);
            assert!(branch.is_branch());
            assert!(!branch.is_leaf());

            // Leaf: has at least one of R, W, X set
            let leaf_r = Entry::new(VALID | READ);
            assert!(leaf_r.is_leaf());
            assert!(!leaf_r.is_branch());

            let leaf_w = Entry::new(VALID | WRITE);
            assert!(leaf_w.is_leaf());

            let leaf_x = Entry::new(VALID | EXECUTE);
            assert!(leaf_x.is_leaf());

            let leaf_rwx = Entry::new(VALID | READ | WRITE | EXECUTE);
            assert!(leaf_rwx.is_leaf());
        }

        #[test]
        fn test_entry_ppn_extraction() {
            // PTE layout: bits [53:10] = PPN
            // PPN[0]: bits [18:10] (9 bits)
            // PPN[1]: bits [27:19] (9 bits)
            // PPN[2]: bits [53:28] (26 bits)

            // Set PPN[0] = 0x1FF (max 9-bit value)
            let entry = Entry::new(0x1FF << 10);
            assert_eq!(entry.extract_ppn(0), 0x1FF);
            assert_eq!(entry.extract_ppn(1), 0);
            assert_eq!(entry.extract_ppn(2), 0);

            // Set PPN[1] = 0x1FF
            let entry = Entry::new(0x1FF << 19);
            assert_eq!(entry.extract_ppn(0), 0);
            assert_eq!(entry.extract_ppn(1), 0x1FF);
            assert_eq!(entry.extract_ppn(2), 0);

            // Set PPN[2] = 0x3FFFFFF (max 26-bit value)
            let entry = Entry::new(0x3FFFFFF_usize << 28);
            assert_eq!(entry.extract_ppn(0), 0);
            assert_eq!(entry.extract_ppn(1), 0);
            assert_eq!(entry.extract_ppn(2), 0x3FFFFFF);
        }

        #[test]
        fn test_entry_ppn_all() {
            // Full 44-bit PPN
            let ppn_all: usize = 0xFFF_FFFFFFFF; // 44 bits all set
            let entry = Entry::new(ppn_all << 10);
            assert_eq!(entry.extract_ppn_all(), ppn_all);
        }

        #[test]
        fn test_entry_set_ppn() {
            let mut entry = Entry::new(VALID | READ);
            let paddr = PhysicalAddr::new(0x8020_0000); // typical kernel address

            entry.set_ppn(paddr);

            // Verify PPN was set correctly (paddr >> 12 = PPN)
            assert_eq!(entry.extract_ppn_all(), 0x8020_0000 >> 12);
            // Verify flags were preserved
            assert!(entry.is_valid());
            assert!(entry.is_read());
        }

        #[test]
        fn test_entry_set_flags_preserves_ppn() {
            let mut entry = Entry::new(0);
            let paddr = PhysicalAddr::new(0x8020_0000);

            entry.set_ppn(paddr);
            entry.set_flags(VALID | READ | WRITE);

            // PPN should still be intact
            assert_eq!(entry.extract_ppn_all(), 0x8020_0000 >> 12);
            assert!(entry.is_valid());
            assert!(entry.is_read());
            assert!(entry.is_write());
        }
    }

    mod virtual_addr_tests {
        use super::*;

        #[test]
        fn test_vaddr_new() {
            let vaddr = VirtualAddr::new(0x8000_0000);
            assert_eq!(vaddr.extract_bits(), 0x8000_0000);
        }

        #[test]
        fn test_vaddr_vpn_extraction() {
            // VirtualAddr layout for Sv39:
            // VPN[0]: bits [20:12] (9 bits) - index into level 0 table
            // VPN[1]: bits [29:21] (9 bits) - index into level 1 table
            // VPN[2]: bits [38:30] (9 bits) - index into level 2 table
            // Offset: bits [11:0] (12 bits)

            // Address: 0x8200_1ABC
            // Binary breakdown:
            //   VPN[2] = (0x8200_1ABC >> 30) & 0x1FF = 2
            //   VPN[1] = (0x8200_1ABC >> 21) & 0x1FF = 16 (0x10)
            //   VPN[0] = (0x8200_1ABC >> 12) & 0x1FF = 1
            //   Offset = 0x8200_1ABC & 0xFFF = 0xABC

            let vaddr = VirtualAddr::new(0x8200_1ABC);
            assert_eq!(vaddr.extract_vpn(2), 2);
            assert_eq!(vaddr.extract_vpn(1), 16);
            assert_eq!(vaddr.extract_vpn(0), 1);
            assert_eq!(vaddr.extract_offset(), 0xABC);
        }

        #[test]
        fn test_vaddr_from_vpn_offset() {
            // Construct address from VPN and offset
            let vpn = (2 << 18) | (16 << 9) | 1; // VPN[2]=2, VPN[1]=16, VPN[0]=1
            let offset = 0xABC;
            let vaddr = VirtualAddr::from(vpn, offset);

            assert_eq!(vaddr.extract_vpn(2), 2);
            assert_eq!(vaddr.extract_vpn(1), 16);
            assert_eq!(vaddr.extract_vpn(0), 1);
            assert_eq!(vaddr.extract_offset(), 0xABC);
        }

        #[test]
        fn test_vaddr_max_vpn_values() {
            // Each VPN field is 9 bits, max value = 511 (0x1FF)
            let max_vpn = (0x1FF << 18) | (0x1FF << 9) | 0x1FF;
            let vaddr = VirtualAddr::from(max_vpn, 0xFFF);

            assert_eq!(vaddr.extract_vpn(2), 0x1FF);
            assert_eq!(vaddr.extract_vpn(1), 0x1FF);
            assert_eq!(vaddr.extract_vpn(0), 0x1FF);
            assert_eq!(vaddr.extract_offset(), 0xFFF);
        }

        #[test]
        fn test_vaddr_alignment() {
            let page_aligned = VirtualAddr::new(0x8000_0000);
            assert!(page_aligned.is_aligned(PAGE_SIZE));

            let not_aligned = VirtualAddr::new(0x8000_0001);
            assert!(!not_aligned.is_aligned(PAGE_SIZE));

            // 2MB (megapage) alignment
            let mega_aligned = VirtualAddr::new(0x8020_0000);
            assert!(mega_aligned.is_aligned(2 * 1024 * 1024));
        }

        #[test]
        fn test_vaddr_zero() {
            let vaddr = VirtualAddr::new(0);
            assert_eq!(vaddr.extract_vpn(0), 0);
            assert_eq!(vaddr.extract_vpn(1), 0);
            assert_eq!(vaddr.extract_vpn(2), 0);
            assert_eq!(vaddr.extract_offset(), 0);
        }
    }

    mod physical_addr_tests {
        use super::*;

        #[test]
        fn test_paddr_new() {
            let paddr = PhysicalAddr::new(0x8000_0000);
            assert_eq!(paddr.extract_bits(), 0x8000_0000);
        }

        #[test]
        fn test_paddr_ppn_extraction() {
            // PhysicalAddr layout for Sv39:
            // PPN[0]: bits [20:12] (9 bits)
            // PPN[1]: bits [29:21] (9 bits)
            // PPN[2]: bits [55:30] (26 bits)
            // Offset: bits [11:0] (12 bits)

            let paddr = PhysicalAddr::new(0x8200_1ABC);
            assert_eq!(paddr.extract_ppn(2), 2);
            assert_eq!(paddr.extract_ppn(1), 16);
            assert_eq!(paddr.extract_ppn(0), 1);
            assert_eq!(paddr.extract_offset(), 0xABC);
        }

        #[test]
        fn test_paddr_ppn_all() {
            let paddr = PhysicalAddr::new(0x8020_1ABC);
            // PPN_all = address >> 12
            assert_eq!(paddr.extract_ppn_all(), 0x8020_1ABC >> 12);
        }

        #[test]
        fn test_paddr_from_ppn_offset() {
            let ppn = 0x80201; // full PPN
            let offset = 0xABC;
            let paddr = PhysicalAddr::from(ppn, offset);

            assert_eq!(paddr.extract_ppn_all(), ppn);
            assert_eq!(paddr.extract_offset(), offset);
            assert_eq!(paddr.extract_bits(), (ppn << 12) | offset);
        }

        #[test]
        fn test_paddr_alignment() {
            let page_aligned = PhysicalAddr::new(0x8000_0000);
            assert!(page_aligned.is_aligned(PAGE_SIZE));

            let not_aligned = PhysicalAddr::new(0x8000_0001);
            assert!(!not_aligned.is_aligned(PAGE_SIZE));
        }

        #[test]
        fn test_paddr_roundtrip() {
            // Verify that from(ppn, offset) and extract work together
            for ppn in [0, 1, 0x80201, 0xFFF_FFFFFFFF_u64 as usize] {
                for offset in [0, 1, 0xFFF] {
                    // Only test valid 44-bit PPNs
                    let ppn = ppn & ((1 << 44) - 1);
                    let paddr = PhysicalAddr::from(ppn, offset);
                    assert_eq!(paddr.extract_ppn_all(), ppn, "PPN mismatch for ppn={ppn:#x}");
                    assert_eq!(
                        paddr.extract_offset(),
                        offset,
                        "Offset mismatch for offset={offset:#x}"
                    );
                }
            }
        }
    }

    mod table_tests {
        use super::*;

        #[test]
        fn test_table_size() {
            assert_eq!(size_of::<Table>(), PAGE_SIZE);
            assert_eq!(size_of::<Entry>(), ENTRY_SIZE);
            assert_eq!(ENTRIES_PER_PAGE, 512);
        }

        #[test]
        fn test_table_new_is_zeroed() {
            let table = Table::new();
            for i in 0..ENTRIES_PER_PAGE {
                assert!(!table.entries[i].is_valid());
            }
        }
    }
}
