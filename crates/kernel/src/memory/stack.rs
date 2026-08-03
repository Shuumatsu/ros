//! Kernel stacks: one static stack for the boot hart, one allocated stack per
//! secondary.
//!
//! This module is the single owner of stack geometry. `boot.S`, `kernel.ld` and
//! [`super::kernel_table`] all consume what is decided here; none of them restates
//! it.
//!
//! # A hart id is not an index
//!
//! **Nothing indexes by hart id here, or anywhere else.** A hart id is an opaque
//! machine identifier, not a small dense number: the privileged spec (§3.1.5,
//! `mhartid`) promises only that ids are unique and that *some* hart has id 0, and
//! real platforms leave gaps — a management core, a disabled core, a cluster number
//! packed into the high bits. OpenSBI keeps a `hart_index2id[]` array and Linux a
//! `__cpuid_to_hartid_map[]` for exactly this reason.
//!
//! An array of slots indexed by hart id costs address space proportional to the
//! largest id rather than to the number of harts, and silently parks the *boot* hart
//! on any machine whose firmware chose one with a large id — before the console
//! exists to say so.
//!
//! # Why the two kinds of stack differ
//!
//! The boot hart needs a stack before it can execute anything interesting, long
//! before there is an allocator or a device tree. So it gets one from the linker: a
//! fixed slot in the image, claimed unconditionally by whichever hart the firmware
//! chose. There is exactly **one**, which is the point — a lone stack needs no index,
//! so there is no id-shaped assumption left to be wrong about.
//!
//! A secondary is a completely different situation. By the time one starts, RAM, the
//! heap and the kernel page table all exist, and *we* decide when it starts. So its
//! stack is allocated here, mapped by [`super::kernel_table`], and handed to it in
//! `a1` by SBI's `opaque` argument; the hart never computes an address of its own.
//! The count then follows the machine instead of a constant, and a hart id of any
//! size or sparsity costs nothing.
//!
//! # Guard pages
//!
//! Stacks grow **down**, so every usable stack sits immediately above an unmapped
//! guard page. An overflow walks off the bottom into the guard and faults, instead of
//! silently eating whatever lies below it — `.bss` for the boot hart, the previous
//! hart's stack for everyone else. Putting the guard *below* is also what makes the
//! stack top the end of the reserved area, so `boot.S` needs one linker symbol and no
//! arithmetic.
//!
//! The guards are holes because [`super::kernel_table`] maps each stack as its own
//! region and never the guards; its `audit_holes` pins that they really are unmapped,
//! since "no entry" is the one property a region list cannot express.

use alloc::vec::Vec;
use core::cell::UnsafeCell;

use paging::sv39::PAGE_SIZE;
use paging::{MemoryAddr, PhysicalAddr, VirtualAddr};
use spin::Once;

use crate::memory::{frame, kernel_va_free_start, layout, virt_to_phys};

/// Unmapped page below each stack, to catch overflow.
pub const GUARD_SIZE: usize = PAGE_SIZE;

/// Usable stack bytes.
pub const SIZE: usize = 64 * 1024;

/// Address-space bytes one stack consumes: its guard page, then the stack.
///
/// Private: it is the size of a *slot*, which is this module's business. Callers want
/// [`Stack`], which hands out the individual addresses.
const STRIDE: usize = GUARD_SIZE + SIZE;

/// One kernel stack: [`SIZE`] usable bytes above a [`GUARD_SIZE`] hole.
///
/// Only two facts are stored; the guard address, the top and the length are derived
/// from them, so the geometry is stated exactly once and a caller cannot be handed a
/// stack whose parts disagree.
#[derive(Clone, Copy, Debug)]
pub struct Stack {
    /// What this stack is for. Labels its region in the page table and the boot log.
    pub name: &'static str,
    /// Physical base of the usable stack — the frame `bottom` maps to.
    pa: PhysicalAddr,
    /// Lowest usable address. `sp` walks down towards it and faults past it.
    ///
    /// Not a direct-map address for a secondary — see the module docs — so it is
    /// emphatically not `phys_to_virt(pa)`, and the types are what keep the two apart
    /// where [`super::kernel_table`] consumes both.
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

/// A secondary hart and the stack allocated for it.
///
/// The pairing is built once, in [`init`], and never recomputed — so there is only
/// ever one answer to "who are the secondaries".
#[derive(Clone, Copy, Debug)]
pub struct Secondary {
    /// The hart this stack was allocated for, as the device tree reports it.
    pub hart: usize,
    pub stack: Stack,
}

/// Backing store for the boot hart's stack.
///
/// Never accessed *through* this item — the hardware writes it via `sp`, which is
/// why the contents sit behind an [`UnsafeCell`]. It exists so that the **size** of
/// the reserved area is declared in exactly one place: here, as [`STRIDE`].
/// `kernel.ld` only *places* it, taking the size from the section, rather than
/// carrying a byte count that silently encodes both [`GUARD_SIZE`] and [`SIZE`].
#[used]
#[unsafe(link_section = ".boot_stack")]
static BOOT_STACK: BootStack = BootStack(UnsafeCell::new([0; STRIDE]));

#[repr(C, align(4096))]
struct BootStack(UnsafeCell<[u8; STRIDE]>);

// SAFETY: the bytes are never read or written through this item — `boot.S` loads
// `sp` from the section bounds and the hardware does the rest. It is `Sync` only so
// it can be a `static`.
unsafe impl Sync for BootStack {}

/// The boot hart's stack, the one `boot.S` takes whole with `la sp, _boot_stack_end`.
///
/// Part of the kernel image, so unlike a secondary's it is direct mapped and its
/// physical address is simply derived.
pub fn boot() -> Stack {
    let bottom = layout::boot_stack_start().add(GUARD_SIZE);
    Stack { name: "boot stack", pa: virt_to_phys(bottom), bottom }
}

/// Assert the linker placed the boot stack exactly where the geometry expects.
///
/// Rust declares the size but the linker chooses the address, and `boot.S` loads `sp`
/// from *its* view of the bounds. If those disagree, the kernel runs on a stack that
/// is not where anyone thinks it is.
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
    // `boot.S` uses `_boot_stack_end` as `sp` directly, so it must be the top this
    // module computes. Two ways of naming one address, pinned rather than assumed.
    assert_eq!(
        boot().top(),
        layout::boot_stack_end(),
        "the boot stack top and `_boot_stack_end` disagree; boot.S would run below its stack"
    );
}

static SECONDARIES: Once<Vec<Secondary>> = Once::new();

/// Allocate one stack per hart in `harts`, above the direct map.
///
/// Call once, on the boot hart, after the frame allocator and heap are up and
/// **before** [`super::kernel_table::init`] — that is what maps them, and a secondary
/// switches to the kernel table before it touches its stack, so the mapping has to be
/// in the table from the start.
///
/// The frames are never released: `Frames` has no destructor and nothing calls
/// `frame::free` on them, which pins them for the kernel's lifetime. A hart's stack is
/// live for as long as the hart is, and nothing stops a hart yet.
///
/// # Why the virtual addresses are not the direct-map ones
///
/// The frames come from the pool, so they are *already* reachable at
/// `phys_to_virt(pa)`. Mapping them a second time above the direct map is what buys
/// the guard page: a hole is impossible inside a contiguous direct-map region. It is
/// the aliasing Linux accepts for `VMAP_STACK`, and sound for the same reason — no
/// hart's `sp` ever points into the alias, so an overflow still walks into the hole.
pub fn init(harts: impl Iterator<Item = usize>) {
    SECONDARIES.call_once(|| {
        let base = kernel_va_free_start();
        harts
            .enumerate()
            .map(|(slot, hart)| {
                let frames = frame::alloc_contiguous(SIZE / PAGE_SIZE)
                    .unwrap_or_else(|| panic!("no contiguous RAM for hart {hart}'s stack"));
                // The slot index is a position in this list and nothing else. It is
                // emphatically not derived from `hart`; see the module docs.
                Secondary {
                    hart,
                    stack: Stack {
                        name: "secondary stack",
                        pa: frames.base(),
                        bottom: base.add(STRIDE * slot + GUARD_SIZE),
                    },
                }
            })
            .collect()
    });
}

/// The secondary harts and their stacks. Empty before [`init`] has run.
pub fn secondaries() -> &'static [Secondary] { SECONDARIES.get().map(Vec::as_slice).unwrap_or(&[]) }

/// Every kernel stack there is, boot hart first.
///
/// The single answer to "what must be mapped". [`super::kernel_table`] walks this
/// rather than assembling the set itself, so a future third kind of stack is one
/// change here and none there.
pub fn all() -> impl Iterator<Item = Stack> {
    core::iter::once(boot()).chain(secondaries().iter().map(|s| s.stack))
}

/// Every guard page, which must all be holes. See [`all`].
pub fn guards() -> impl Iterator<Item = VirtualAddr> { all().map(|stack| stack.guard()) }

/// Print the stack geometry.
///
/// `memory::report_layout` prints where the linker put the boot stack, an image-layout
/// fact; the sizes, the guards and the secondaries are this module's.
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
