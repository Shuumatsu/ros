//! Compile-time page table used until the final kernel table is ready.

use paging::sv39::{PAGE_OFFSET_BITS, PPN_BITS, PteFlags};
use paging::{PhysicalAddr, Satp, Table};

use super::direct_map::VA_OFFSET;

/// Blanket rights over everything, with `A`/`D` pre-set so the hardware walker
/// never writes back into a table that lives in `.rodata`. Deliberately temporary:
/// [`super::kernel_table`] replaces every one of these leaves with per-section
/// permissions as soon as there is an allocator to build a real tree.
const BOOT: PteFlags = PteFlags::READ_WRITE_EXECUTE.union(PteFlags::ACCESS).union(PteFlags::DIRTY);

/// The table the architecture entry installs to reach high virtual addresses.
///
/// A `static` with a `const` initializer, so it is bytes in the image rather than
/// code that has to run somewhere — which matters, because the code that would run
/// it could not reach its own `&'static`s yet.
#[used]
pub(crate) static TABLE: Table = Table::identity_and_offset(VA_OFFSET, BOOT);

/// The `satp` for [`TABLE`], in the two pieces assembly can assemble it from.
///
/// [`TABLE`]'s physical address is a link-time fact, so the entry cannot be handed a
/// finished register value: it recovers the address PC-relatively and folds it in
/// with `srli`+`or`. That recipe is a second encoding of [`Satp::with_root`], and
/// assembly cannot call the first one — so both constants live here, next to the
/// assertion that pins them to it.
pub(crate) const SATP_TEMPLATE: usize = Satp::sv39(PhysicalAddr::new(0), 0).bits();
/// Right-shift that turns a page-aligned root address into `satp.PPN`.
pub(crate) const SATP_ROOT_SHIFT: usize = PAGE_OFFSET_BITS;

const _: () = {
    // The recipe is linear in the address, so one root per PPN bit covers every
    // field boundary it could get wrong.
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
