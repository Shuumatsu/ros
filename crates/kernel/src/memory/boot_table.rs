//! Compile-time page table used until the final kernel table is ready.

use mmu::{PAGE_OFFSET_BITS, PPN_BITS, PhysicalAddr, PteFlags, Satp, Scheme, Table};

use super::{KernelScheme, direct_map};

/// Temporary blanket rights with `A`/`D` set to keep the walker from writing `.rodata`.
const BOOT: PteFlags = PteFlags::READ_WRITE_EXECUTE.union(PteFlags::ACCESS).union(PteFlags::DIRTY);

/// Compile-time table used by the architecture entry.
///
/// Its high half covers only the direct map, leaving chosen kernel VAs unmapped.
#[used]
pub static TABLE: Table =
    Table::identity_and_offset::<KernelScheme>(direct_map::VA_OFFSET, direct_map::SPAN, BOOT);

/// The scheme and ASID bits assembly combines with [`TABLE`]'s shifted physical address.
///
/// The assertion below verifies assembly's `srli`/`or` encoding against [`Satp::new`].
pub const SATP_TEMPLATE: usize = Satp::new(KernelScheme::MODE, 0, PhysicalAddr::new(0)).bits();
pub const SATP_ROOT_SHIFT: usize = PAGE_OFFSET_BITS;

const _: () = {
    let mut bit = 0;
    while bit < PPN_BITS {
        let root = PhysicalAddr::from_ppn(1 << bit);
        assert!(
            SATP_TEMPLATE | (root.bits() >> SATP_ROOT_SHIFT)
                == Satp::new(KernelScheme::MODE, 0, root).bits(),
            "the boot entry's srli+or no longer reproduces Satp::new"
        );
        bit += 1;
    }
};
