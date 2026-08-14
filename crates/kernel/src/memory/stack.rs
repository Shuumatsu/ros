//! Kernel stacks: one from the linker for the boot hart, one allocated per secondary.
//!
//! The single owner of stack geometry; the boot entry, `kernel.ld` and
//! [`super::kernel_table`] consume what is decided here.
//!
//! The boot hart needs a stack before there is an allocator, so it gets a fixed slot in
//! the image — exactly one, so no index is involved. A secondary starts when we choose,
//! after the allocator and page table exist, so its stack is allocated here and handed
//! over through SBI's `opaque`; the hart computes no address of its own.
//!
//! **Nothing indexes by hart id.** Ids are only promised to be unique, and real platforms
//! leave gaps, so an array indexed by id costs address space proportional to the largest
//! id and mis-slots the boot hart on machines that pick a large one.
//!
//! Stacks grow down, so each sits above an unmapped guard page: an overflow faults instead
//! of eating `.bss` or the next hart's stack. Guard *below* also makes the stack top the
//! end of the reserved area, the one symbol the boot entry needs.

use alloc::vec::Vec;
use core::cell::UnsafeCell;

use paging::sv39::PAGE_SIZE;
use paging::{MemoryAddr, PhysicalAddr, VirtualAddr};
use spin::Once;

use crate::memory::{frame, kernel_va, layout, virt_to_phys};

/// Unmapped page below each stack, to catch overflow.
pub const GUARD_SIZE: usize = PAGE_SIZE;

/// Usable stack bytes.
pub const SIZE: usize = 64 * 1024;

/// Address-space bytes one stack consumes: its guard page, then the stack. Private —
/// callers want [`Stack`], which hands out the individual addresses.
const STRIDE: usize = GUARD_SIZE + SIZE;
const _: () = assert!(STRIDE % 16 == 0, "kernel stack top must be 16-byte aligned");

/// One kernel stack: [`SIZE`] usable bytes above a [`GUARD_SIZE`] hole.
///
/// Two facts stored, the rest derived, so no caller can be handed a stack whose parts
/// disagree.
#[derive(Clone, Copy, Debug)]
pub struct Stack {
    /// What this stack is for. Labels its region in the page table and the boot log.
    pub name: &'static str,
    /// Physical base of the usable stack — the frame `bottom` maps to.
    pa: PhysicalAddr,
    /// Lowest usable address. `sp` walks down towards it and faults past it.
    ///
    /// Not `phys_to_virt(pa)`: a secondary's stack is deliberately mapped outside the
    /// direct map (see [`init`]), and the types keep the two apart.
    bottom: VirtualAddr,
}

impl Stack {
    /// First address of the guard page, which must never be mapped.
    pub fn guard(&self) -> VirtualAddr { self.bottom.sub(GUARD_SIZE) }

    /// Lowest usable stack address.
    pub fn bottom(&self) -> VirtualAddr { self.bottom }

    /// One past the highest — the value a starting hart loads into `sp`.
    pub fn top(&self) -> VirtualAddr { self.bottom.add(SIZE) }

    /// Physical base of the usable stack.
    pub fn pa(&self) -> PhysicalAddr { self.pa }

    /// Usable bytes, i.e. what [`super::kernel_table`] must map.
    pub fn len(&self) -> usize { SIZE }
}

/// A secondary hart and the stack allocated for it. Built once, in [`init`], so there
/// is one answer to "who are the secondaries".
#[derive(Clone, Copy, Debug)]
pub struct Secondary {
    /// The hart this stack was allocated for, as the device tree reports it.
    pub hart: usize,
    pub stack: Stack,
}

/// Backing store for the boot hart's stack, never accessed through this item — the
/// hardware writes it via `sp`, hence the [`UnsafeCell`].
///
/// It exists so the reserved area's size is declared once, here; `kernel.ld` only places
/// the section rather than carrying a byte count that encodes both constants.
#[used]
#[unsafe(link_section = ".boot_stack")]
static BOOT_STACK: BootStack = BootStack(UnsafeCell::new([0; STRIDE]));

#[repr(C, align(4096))]
struct BootStack(UnsafeCell<[u8; STRIDE]>);

// SAFETY: the bytes are never read or written through this item — the boot entry loads
// `sp` from the section bounds and the hardware does the rest. It is `Sync` only so
// it can be a `static`.
unsafe impl Sync for BootStack {}

/// The boot hart's stack, taken whole by the architecture entry.
///
/// Part of the kernel image, so unlike a secondary's it is direct mapped and its
/// physical address is simply derived.
pub fn boot() -> Stack {
    let bottom = layout::boot_stack_start().add(GUARD_SIZE);
    Stack { name: "boot stack", pa: virt_to_phys(bottom), bottom }
}

/// Assert the linker placed the boot stack where the geometry expects.
///
/// Rust declares the size, the linker chooses the address; if they disagree the kernel
/// runs on a stack that is not where anyone thinks it is.
pub fn check_layout() {
    let span = layout::boot_stack_end().sub_addr(layout::boot_stack_start());
    assert_eq!(
        span, STRIDE,
        "the .boot_stack section spans {span:#x} bytes but the geometry needs {STRIDE:#x}; \
         kernel.ld is not placing it as a whole"
    );
    assert!(
        layout::boot_stack_start().is_aligned(PAGE_SIZE),
        "the boot stack must be page aligned or its guard page will not line up"
    );
    // The architecture entry uses `_boot_stack_end` as `sp` directly.
    assert_eq!(
        boot().top(),
        layout::boot_stack_end(),
        "the boot stack top and `_boot_stack_end` disagree"
    );
}

static SECONDARIES: Once<Vec<Secondary>> = Once::new();

/// Allocate one stack per hart in `harts`, at addresses from [`kernel_va`].
///
/// Call once, on the boot hart, after the frame allocator and heap are up and **before**
/// [`super::kernel_table::init`]: a secondary switches to the kernel table before it
/// touches its stack, so the mapping must already be there. Frames are
/// [`leak`](frame::Frames::leak)ed, since a stack lives as long as its hart.
///
/// The frames are already reachable at `phys_to_virt(pa)`; mapping them again above the
/// direct map is what buys the guard page, since a hole is impossible inside a contiguous
/// direct-map region. Same aliasing Linux accepts for `VMAP_STACK`, sound for the same
/// reason: no `sp` points into the alias.
///
/// Slots are reserved whole and one at a time, so there is no index to compute wrong.
///
/// # Panics
///
/// If the list has already been built. `Once` alone would keep the first one, so a second
/// call would leave its harts with no stacks and say nothing — and `cpu` starts exactly
/// the harts named here.
pub fn init(harts: impl Iterator<Item = usize>) {
    assert!(SECONDARIES.get().is_none(), "stack::init called twice; the stacks are already built");

    SECONDARIES.call_once(|| {
        harts
            .map(|hart| {
                let frames = frame::alloc_contiguous(SIZE / PAGE_SIZE)
                    .unwrap_or_else(|| panic!("no contiguous RAM for hart {hart}'s stack"));
                // Buddy rounding would otherwise strand frames the stack never uses.
                assert_eq!(
                    frames.len(),
                    SIZE,
                    "hart {hart}'s stack asked for {SIZE:#x} bytes and got {:#x}; \
                     kernel stack SIZE is not a power-of-two multiple of the page size",
                    frames.len()
                );
                let slot = kernel_va::reserve(STRIDE, PAGE_SIZE);
                Secondary {
                    hart,
                    stack: Stack {
                        name: "secondary stack",
                        pa: frames.leak(),
                        bottom: slot.add(GUARD_SIZE),
                    },
                }
            })
            .collect()
    });
}

/// The secondary harts and their stacks. Empty before [`init`] has run.
pub fn secondaries() -> &'static [Secondary] { SECONDARIES.get().map(Vec::as_slice).unwrap_or(&[]) }

/// Every kernel stack there is, boot hart first — the single answer to "what must be
/// mapped", so a third kind of stack is one change here and none in
/// [`super::kernel_table`].
pub fn all() -> impl Iterator<Item = Stack> {
    core::iter::once(boot()).chain(secondaries().iter().map(|s| s.stack))
}

/// Every guard page, which must all be holes. See [`all`].
pub fn guards() -> impl Iterator<Item = VirtualAddr> { all().map(|stack| stack.guard()) }

/// Print the stack geometry: sizes, guards and secondaries. Where the linker put the
/// boot stack is [`super::layout::report`]'s.
pub fn report() {
    let secondaries = secondaries();
    println!(
        "[memory] stacks: 1 boot + {} secondary x {} (each above a {} guard)",
        secondaries.len(),
        crate::utils::ByteSize(SIZE),
        crate::utils::ByteSize(GUARD_SIZE)
    );
    // The span only. Which hart got which stack is `cpu::start_secondaries`' to
    // report, and it does, one line per hart as it starts them.
    if let (Some(first), Some(last)) = (secondaries.first(), secondaries.last()) {
        println!(
            "[memory]   secondary stacks at {:#x}..{:#x}",
            first.stack.guard(),
            last.stack.top()
        );
    }
}
