//! Compile-time page table used until the final kernel table is ready.

use paging::sv39::{PteFlags, ROOT_LEVEL, page_size_at};
use paging::{PhysicalAddr, Satp, Table, VirtualAddr};

use super::direct_map::{ROOT_ENTRIES, VA_OFFSET};

const GIGAPAGE: usize = page_size_at(ROOT_LEVEL);
const BOOT: PteFlags = PteFlags::READ_WRITE_EXECUTE.union(PteFlags::ACCESS).union(PteFlags::DIRTY);

const fn table() -> Table {
    let mut table = Table::new();
    let mut index = 0;
    while index < ROOT_ENTRIES {
        let pa = PhysicalAddr::new(index * GIGAPAGE);
        table.map_gigapage(VirtualAddr::new(index * GIGAPAGE), pa, BOOT);
        table.map_gigapage(VirtualAddr::new(VA_OFFSET + index * GIGAPAGE), pa, BOOT);
        index += 1;
    }
    table
}

#[used]
pub(crate) static TABLE: Table = table();

pub(crate) const SATP_TEMPLATE: usize = Satp::sv39(PhysicalAddr::new(0), 0).bits();
