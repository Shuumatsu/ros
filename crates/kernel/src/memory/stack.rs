//! Kernel stacks: one from the linker for the boot hart, one allocated on request for
//! every other.
//!
//! The single owner of stack *geometry* — the size, the guard page, and where in the
//! address space a stack goes; the boot entry, `kernel.ld` and [`super::kernel_table`]
//! consume what is decided here. Who gets which stack is not a memory fact and is not
//! here: [`crate::cpu`] asks for one per hart it means to start and keeps the pairing. All
//! this module keeps is the list of stacks that exist, because that is the list
//! [`super::kernel_table`] must map.
//!
//! The boot hart needs a stack before there is an allocator, so it gets a fixed slot in
//! the image — exactly one, so no index is involved. A secondary starts when we choose,
//! after the allocator and page table exist, so its stack comes from [`alloc`] and is
//! handed over through SBI's `opaque`; the hart computes no address of its own.
//!
//! Stacks grow down, so each sits above an unmapped guard page: an overflow faults instead
//! of eating `.bss` or the next hart's stack. Guard *below* also makes the stack top the
//! end of the reserved area, the one symbol the boot entry needs.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

use heapless::Vec;

use mmu::PAGE_SIZE;
use mmu::{MemoryAddr, PhysicalAddr, VirtualAddr};

use super::direct_map::virt_to_phys;
use super::{frame, kernel_va, layout};
use crate::arch;
use crate::cpu::MAX_CPUS;
use crate::sync::IrqMutex;
use crate::utils::ByteSize;

/// Unmapped page below each stack, to catch overflow.
const GUARD_SIZE: usize = PAGE_SIZE;

/// Usable stack bytes.
const SIZE: usize = 64 * 1024;

/// Address-space bytes one stack consumes: its guard page, then the stack. Callers want
/// [`Stack`], which hands out the individual addresses.
const STRIDE: usize = GUARD_SIZE + SIZE;
const _: () = assert!(STRIDE.is_multiple_of(16), "kernel stack top must be 16-byte aligned");

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
    /// direct map (see [`alloc`]), and the types keep the two apart.
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

    /// Usable bytes, i.e. what [`super::kernel_table`] must map. Not `len`, for
    /// [`frame::Frames::bytes`]'s reason: the guard page makes "how long is a stack" a
    /// question with two answers, and this is the one that gets mapped.
    pub fn bytes(&self) -> usize { SIZE }
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
/// physical address is simply derived. Not in [`ALLOCATED`]: it exists before that list
/// can, and [`all`] puts it back at the front.
pub fn boot() -> Stack {
    let bottom = layout::boot_stack_start().add(GUARD_SIZE);
    Stack { name: "boot stack", pa: virt_to_phys(bottom), bottom }
}

/// Assert the linker placed the boot stack where the geometry expects.
///
/// Rust declares the size, the linker chooses the address; if they disagree the kernel
/// runs on a stack that is not where anyone thinks it is.
pub fn check() {
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

/// Stacks [`alloc`] can hand out: one per hart [`crate::cpu`] may start, less the boot
/// hart's, which the linker provides. A stack taken *after* the table is built is absent by
/// design — see [`super::alloc_kernel_stack`], which maps its own and needs no list.
///
/// A stack exists to be the one a hart runs on, so the hart cap *is* this cap; taking the
/// number from `cpu` rather than restating it is the difference between a full pool and
/// two numbers that can disagree. Fixed capacity, so [`all`] can snapshot the list without
/// allocating — a heap `Vec` here would grow inside this lock and take the heap's, which is
/// the one nesting [`super::heap`] exists to avoid.
const MAX_ALLOCATED: usize = MAX_CPUS - 1;

/// Every stack [`alloc`] has handed out, which with [`boot`] is everything
/// [`super::kernel_table`] must map.
///
/// A list rather than a count, because the mapper needs each stack's own `va` and `pa`:
/// the addresses are not a stride apart in both spaces at once.
static ALLOCATED: IrqMutex<Vec<Stack, MAX_ALLOCATED>> = IrqMutex::new(Vec::new());

/// Set once the kernel page table has been built from [`all`], after which a new stack
/// would be memory no table maps.
static SEALED: AtomicBool = AtomicBool::new(false);

/// Allocate a kernel stack for a hart, to be mapped when the kernel table is built.
///
/// Call on the boot hart, after [`frame::init`] and **before**
/// [`super::kernel_table::init`]: a secondary switches to the kernel table before it
/// touches its stack, so the mapping must already be there. That ordering is what [`seal`]
/// enforces, since the alternative is a hart faulting on its first push with no diagnosis
/// available.
///
/// The heap is not needed, which is why this can be the first thing to happen after the
/// frame allocator: the list below is fixed-capacity.
///
/// A stack for something that does not exist yet when the table is built — a process, whose
/// kernel stack is its own — comes from [`super::alloc_kernel_stack`] instead.
///
/// # Panics
///
/// If the pool cannot produce [`SIZE`] contiguous bytes, or if the kernel page table has
/// already been built.
pub fn alloc(name: &'static str) -> Stack {
    assert!(
        !SEALED.load(Ordering::Acquire),
        "stack::alloc after the kernel page table was built; '{name}' would be mapped by \
         nothing and fault on its first push"
    );

    let stack = reserve(name);
    ALLOCATED
        .with(|allocated| allocated.push(stack))
        .expect("one stack per hart, and cpu::MAX_CPUS bounds the harts");
    stack
}

/// Take frames and an address for one stack, recording it nowhere.
///
/// The whole of what [`alloc`] and [`super::alloc_kernel_stack`] have in common: same geometry,
/// same frames, same kind of address, and only who installs the mapping differs. Frames are
/// [`leak`](frame::Frames::leak)ed, since a stack lives as long as whatever runs on it.
///
/// The frames are already reachable at `phys_to_virt(pa)`; mapping them again above the direct
/// map is what buys the guard page, since a hole is impossible inside a contiguous direct-map
/// region. Same aliasing Linux accepts for `VMAP_STACK`, sound for the same reason: no `sp`
/// points into the alias.
///
/// One stack per call, reserved whole, so there is no index to compute wrong and no way for a
/// caller to be handed the same stack twice.
pub(in crate::memory) fn reserve(name: &'static str) -> Stack {
    let frames = frame::alloc_contiguous(SIZE / PAGE_SIZE)
        .unwrap_or_else(|| panic!("no contiguous RAM for a {SIZE:#x}-byte '{name}'"));
    // Buddy rounding would otherwise strand frames the stack never uses.
    assert_eq!(
        frames.bytes(),
        SIZE,
        "'{name}' asked for {SIZE:#x} bytes and got {:#x}; kernel stack SIZE is not a \
         power-of-two multiple of the page size",
        frames.bytes()
    );

    let slot = kernel_va::reserve(STRIDE, PAGE_SIZE);
    Stack { name, pa: frames.leak(), bottom: slot.add(GUARD_SIZE) }
}

/// Every stack that exists before the kernel table is built, boot hart first — the single
/// answer to "what that table must map", so a third kind of hart stack is one change here and
/// none in [`super::kernel_table`].
///
/// Snapshotted rather than borrowed: the list is behind a lock, and handing out a reference
/// into it would mean holding that lock for as long as the caller walks the page table.
pub fn all() -> impl Iterator<Item = Stack> {
    let allocated = ALLOCATED.with(|allocated| allocated.clone());
    core::iter::once(boot()).chain(allocated)
}

/// Every guard page, which must all be holes. See [`all`].
pub fn guards() -> impl Iterator<Item = VirtualAddr> { all().map(|stack| stack.guard()) }

/// Refuse further [`alloc`] calls, because the table that maps stacks is being built.
///
/// Called by [`super::kernel_table::init`] *before* it reads [`all`], so the list that
/// becomes the region set is provably the final one. Not a `Once` over the list: the list is
/// built one call at a time by whoever needs a stack, and what has to be pinned is the
/// moment it stops growing.
pub fn seal() { SEALED.store(true, Ordering::Release); }

/// Print the stack geometry: sizes, guards and how many exist. Where the linker put the
/// boot stack is [`super::layout::report`]'s.
pub fn report() {
    let allocated = ALLOCATED.with(|allocated| allocated.clone());
    println!(
        "[memory] stacks: 1 boot + {} allocated x {} (each above a {} guard)",
        allocated.len(),
        ByteSize(SIZE),
        ByteSize(GUARD_SIZE)
    );
    // The span only. Which hart got which stack is `cpu::start_secondaries`' to report,
    // and it does, one line per hart as it starts them.
    if let (Some(first), Some(last)) = (allocated.first(), allocated.last()) {
        println!("[memory]   allocated stacks at {:#x}..{:#x}", first.guard(), last.top());
    }
}

/// Prove `stack` is really usable: run a kernel context on it and require the stack pointer it
/// reports to be inside it.
///
/// For [`super::alloc_kernel_stack`]'s benefit. A stack the live table failed to map faults on
/// its first push, which surfaces as a fault inside a context switch with nothing pointing back
/// at the allocation; this asks the question where the answer is still cheap.
///
/// Spends the stack it is given: [`kernel_va`] is a watermark with no free, so a stack used for
/// this is not handed on.
pub fn self_test(stack: Stack) {
    let observed = arch::context::self_test(stack.top());
    assert!(
        stack.bottom() < observed && observed <= stack.top(),
        "'{}' reported a stack pointer of {observed:#x}, outside its {:#x}..{:#x}",
        stack.name,
        stack.bottom(),
        stack.top()
    );
    println!(
        "[memory] '{}' at {:#x}..{:#x}: a kernel context ran on it (sp {observed:#x}) and \
         switched back",
        stack.name,
        stack.bottom(),
        stack.top()
    );
}
