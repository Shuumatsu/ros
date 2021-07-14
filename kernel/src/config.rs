use crate::utils::KILOBYTE;

pub const USER_STACK_SIZE: usize = 8 * KILOBYTE;

pub const PAGE_SIZE: usize = 4 * KILOBYTE;
pub const ENTRY_SIZE: usize = 8;
pub const ENTRIES_PER_PAGE: usize = PAGE_SIZE / ENTRY_SIZE;
pub const PAGE_SIZE_BITS: usize = 0xc;

pub const TRAMPOLINE: usize = usize::MAX - PAGE_SIZE + 1;
pub const TRAP_CONTEXT: usize = TRAMPOLINE - PAGE_SIZE;

pub const CLOCK_FREQ: usize = 12500000;
