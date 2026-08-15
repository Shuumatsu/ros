//! Compile-time page table used until the final kernel table is ready.

use paging::sv39::{PAGE_OFFSET_BITS, PPN_BITS, PteFlags};
use paging::{PhysicalAddr, Satp, Table};

use super::direct_map::{DIRECT_MAP_SPAN, VA_OFFSET};

/// Blanket rights, `A`/`D` pre-set so the walker never writes back into a table living in
/// `.rodata`. Temporary: [`super::kernel_table`] replaces every leaf once there is an
/// allocator to build a real tree with.
const BOOT: PteFlags = PteFlags::READ_WRITE_EXECUTE.union(PteFlags::ACCESS).union(PteFlags::DIRTY);

/// The table the architecture entry installs to reach high virtual addresses.
///
/// A `const` initializer, so it is bytes in the image rather than code — the code that
/// would build it could not reach its own `&'static`s yet.
///
/// The high half covers the direct map's window and no more, so the addresses
/// [`super::kernel_va`] hands out are unmapped here exactly as they are in the finished
/// table — a stray write to one faults instead of landing on whatever physical memory the
/// blanket mapping would have aliased.
#[used]
pub(crate) static TABLE: Table = Table::identity_and_offset(VA_OFFSET, DIRECT_MAP_SPAN, BOOT);

/// The `satp` for [`TABLE`], in the two pieces assembly can assemble it from.
///
/// `satp` needs [`TABLE`]'s physical address, and before translation is on a PC-relative
/// load is the only way to name it, so the entry recovers it there and folds it in with
/// `srli`+`or` — a second encoding of [`Satp::sv39`], which assembly cannot call, so both
/// constants live next to the assertion that pins them to it.
pub(crate) const SATP_TEMPLATE: usize = Satp::sv39(PhysicalAddr::new(0), 0).bits();
/// Right-shift that turns a page-aligned root address into `satp.PPN`.
pub(crate) const SATP_ROOT_SHIFT: usize = PAGE_OFFSET_BITS;

const _: () = {
    // Linear in the address, so one root per PPN bit covers every field boundary.
    let mut bit = 0;
    while bit < PPN_BITS {
        let root = PhysicalAddr::from_ppn(1 << bit);
        assert!(
            SATP_TEMPLATE | (root.bits() >> SATP_ROOT_SHIFT) == Satp::sv39(root, 0).bits(),
            "the boot entry's srli+or no longer reproduces Satp::sv39"
        );
        bit += 1;
    }
};
