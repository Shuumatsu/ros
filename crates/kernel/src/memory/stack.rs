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

use core::cell::UnsafeCell;

use paging::sv39::PAGE_SIZE;

use crate::memory::layout;

/// Unmapped page below each hart's stack, to catch overflow.
pub const GUARD_SIZE: usize = PAGE_SIZE;

/// Usable stack bytes per hart.
pub const SIZE: usize = 64 * 1024;

/// Address-space bytes each hart consumes: its guard page, then its stack.
pub const STRIDE: usize = GUARD_SIZE + SIZE;

/// Harts the kernel reserves stack space for.
pub const MAX_HARTS: usize = 16;

/// Backing store for every hart's stack slot.
///
/// Never accessed *through* this item — the hardware writes it via `sp`, which is
/// why the contents sit behind an [`UnsafeCell`]. It exists so that the **size** of
/// the stack area is declared in exactly one place: here, as `STRIDE * MAX_HARTS`.
/// `kernel.ld` only *places* it, taking the size from the section.
///
/// The alternative was a hand-computed byte count in the linker script, and that is
/// a composite magic number: `0x110000` silently encodes all three of [`MAX_HARTS`],
/// [`GUARD_SIZE`] and [`SIZE`], in a form nothing can check and no reader can
/// verify. Changing [`SIZE`] would have left the total stale and quietly changed the
/// hart count instead.
#[used]
#[unsafe(link_section = ".hart_stacks")]
static HART_STACKS: HartStacks = HartStacks(UnsafeCell::new([0; STRIDE * MAX_HARTS]));

#[repr(C, align(4096))]
struct HartStacks(UnsafeCell<[u8; STRIDE * MAX_HARTS]>);

// SAFETY: the bytes are never read or written through this item — `boot.S` derives
// `sp` from the section bounds and the hardware does the rest. It is `Sync` only so
// it can be a `static`.
unsafe impl Sync for HartStacks {}

/// The stride, as a word `boot.S` can load.
///
/// It computes `sp` from this and derives the hart limit from the linker's area
/// bounds — the same two facts, the same way, so neither side restates the other.
#[used]
#[unsafe(no_mangle)]
static HART_STACK_STRIDE: usize = STRIDE;

/// How many harts there is stack space for.
///
/// `boot.S` parks any hart whose id reaches this rather than letting it run on a
/// neighbour's stack. It derives the same number as `span / STRIDE` from the section
/// bounds, which is equal by construction — the section *is* `STRIDE * MAX_HARTS`
/// bytes — and [`check_layout`] pins that equality so a linker script that placed the
/// section wrongly cannot go unnoticed.
pub fn max_harts() -> usize {
    MAX_HARTS
}

/// Assert the linker placed the stack section exactly where the geometry expects.
///
/// Cheap, and it closes the one gap in the arrangement: Rust declares the size but
/// the linker chooses the address, and `boot.S` computes `sp` from *its* view of the
/// bounds. If those disagree, every hart runs on a stack that is not where anyone
/// thinks it is.
pub fn check_layout() {
    let span = layout::kernel_stack_end() - layout::kernel_stack_start();
    assert_eq!(
        span,
        STRIDE * MAX_HARTS,
        "the .hart_stacks section spans {span:#x} bytes but the geometry needs {:#x}; \
         kernel.ld is not placing it as a whole",
        STRIDE * MAX_HARTS
    );
    assert_eq!(
        layout::kernel_stack_start() % PAGE_SIZE,
        0,
        "the stack area must be page aligned or the guard pages will not line up"
    );
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
