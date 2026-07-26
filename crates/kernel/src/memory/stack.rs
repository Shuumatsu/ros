//! Per-hart kernel stack geometry.
//!
//! Kernel stacks grow **down**, so each hart's usable stack sits immediately above
//! an unmapped guard page. An overflow walks off the bottom into the guard and
//! faults, instead of silently eating whatever lies below — `.bss` for hart 0, the
//! previous hart's stack for everyone else. The guards are holes because
//! [`super::kernel_table`] maps each stack as its own region and never the guards.
//!
//! Layout for hart `h`:
//!
//! ```text
//!   guard  [start + STRIDE*h,            start + STRIDE*h + GUARD_SIZE)
//!   stack  [start + STRIDE*h + GUARD_SIZE, start + STRIDE*(h+1))
//! ```
//!
//! which makes the stack top — and therefore `sp` — simply
//! `start + STRIDE*(h+1)`, so `boot.S` needs only the stride.
//!
//! # Who owns what
//!
//! This module owns the subdivision; `kernel.ld` reserves only the *total* size of
//! the area. [`max_harts`] is therefore **derived** from that total rather than
//! declared, so resizing the area changes the hart count and nothing can hold a
//! stale copy.
//!
//! It has to be this way round. Rust can read `_kernel_stack_start` and
//! `_kernel_stack_end` because their values sit near the code, but it cannot read a
//! small absolute linker symbol at all: the reference is PC-relative and a value
//! like 4096 is far outside a `R_RISCV_PCREL_HI20` displacement. `boot.S` has no
//! such limitation and derives the same numbers the same way, reading [`STRIDE`]
//! from the word exported below.

use paging::sv39::PAGE_SIZE;

use crate::memory::layout;

/// Unmapped page below each hart's stack, to catch overflow.
pub const GUARD_SIZE: usize = PAGE_SIZE;

/// Usable stack bytes per hart.
pub const SIZE: usize = 64 * 1024;

/// Address-space bytes each hart consumes: its guard page, then its stack.
pub const STRIDE: usize = GUARD_SIZE + SIZE;

/// The stride, as a word `boot.S` can load.
///
/// It computes `sp` from this and derives the hart limit from the linker's area
/// bounds — the same two facts, the same way, so neither side restates the other.
#[used]
#[unsafe(no_mangle)]
static HART_STACK_STRIDE: usize = STRIDE;

/// Total bytes the linker reserved for stacks.
fn area_size() -> usize {
    layout::kernel_stack_end() - layout::kernel_stack_start()
}

/// How many harts there is stack space for.
///
/// Derived from the reserved area, never declared. `boot.S` parks any hart whose id
/// reaches this rather than letting it run on a neighbour's stack.
pub fn max_harts() -> usize {
    area_size() / STRIDE
}

/// `[bottom, top)` of hart `hart`'s usable stack, excluding its guard page.
///
/// # Panics
/// If `hart` has no reserved stack — the same condition `boot.S` parks on, so
/// reaching it here means the two disagree.
pub fn stack(hart: usize) -> (usize, usize) {
    assert!(
        hart < max_harts(),
        "hart {hart} has no reserved stack (space for {} harts)",
        max_harts()
    );
    let bottom = layout::kernel_stack_start() + STRIDE * hart + GUARD_SIZE;
    (bottom, bottom + SIZE)
}

/// First address of hart `hart`'s guard page, which must never be mapped.
pub fn guard(hart: usize) -> usize {
    assert!(hart < max_harts(), "hart {hart} has no reserved stack");
    layout::kernel_stack_start() + STRIDE * hart
}
