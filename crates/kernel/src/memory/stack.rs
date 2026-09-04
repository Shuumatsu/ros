//! Kernel stack geometry and allocation.
//!
//! Each downward-growing stack sits above an unmapped guard page. The boot stack is linked into
//! the image; later stacks use allocated frames at reserved kernel virtual addresses.

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
use crate::utils::{ByteSize, KIB};

const GUARD_SIZE: usize = PAGE_SIZE;

pub const SIZE: usize = 64 * KIB;

const STRIDE: usize = GUARD_SIZE + SIZE;

// Every stack begins on a page boundary and spans whole pages, so its top lands on one too, and
// a page is a whole number of the alignment the ABI requires of `sp`.
const _: () = assert!(
    SIZE.is_multiple_of(PAGE_SIZE) && PAGE_SIZE.is_multiple_of(arch::STACK_ALIGN),
    "a kernel stack top must be aligned as the ABI requires of `sp`"
);

/// A kernel stack above an unmapped guard page.
#[derive(Clone, Copy, Debug)]
pub struct Stack {
    pub name: &'static str,
    pa: PhysicalAddr,
    /// Lowest usable VA; allocated stacks are not addressed through their direct-map alias.
    bottom: VirtualAddr,
}

impl Stack {
    pub fn guard(&self) -> VirtualAddr { self.bottom.sub(GUARD_SIZE) }

    pub fn bottom(&self) -> VirtualAddr { self.bottom }

    pub fn top(&self) -> VirtualAddr { self.bottom.add(SIZE) }

    pub fn pa(&self) -> PhysicalAddr { self.pa }
}

/// Linker-placed boot stack backing storage, accessed only through `sp`.
#[used]
#[unsafe(link_section = ".boot_stack")]
static BOOT_STACK: BootStack = BootStack(UnsafeCell::new([0; STRIDE]));

#[repr(C, align(4096))]
struct BootStack(UnsafeCell<[u8; STRIDE]>);

// SAFETY: the bytes are accessed only through the boot hart's `sp`.
unsafe impl Sync for BootStack {}

/// Return the direct-mapped boot stack.
pub fn boot() -> Stack {
    let bottom = layout::boot_stack_start().add(GUARD_SIZE);
    Stack { name: "boot stack", pa: virt_to_phys(bottom), bottom }
}

/// Verify the linker-provided boot stack geometry.
///
/// # Panics
///
/// Panics if the section size, alignment, or top differs from the Rust geometry.
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
    assert_eq!(
        boot().top(),
        layout::boot_stack_end(),
        "the boot stack top and `_boot_stack_end` disagree"
    );
}

/// Pre-table stacks exclude the linker-provided boot stack.
const MAX_ALLOCATED: usize = MAX_CPUS - 1;

static ALLOCATED: IrqMutex<Vec<Stack, MAX_ALLOCATED>> = IrqMutex::new(Vec::new());

/// Prevents allocations after the table snapshots [`all`].
static SEALED: AtomicBool = AtomicBool::new(false);

/// Allocate a kernel stack for a hart, to be mapped when the kernel table is built.
///
/// Call on the boot hart after frame initialization and before the kernel table is built.
///
/// # Panics
///
/// Panics if contiguous frames are unavailable or the table has already been built.
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

/// Reserve backing frames and a kernel VA without recording or mapping the stack.
///
/// Mapping above the direct map permits an unmapped guard. No `sp` may use the direct-map alias.
pub(in crate::memory) fn reserve(name: &'static str) -> Stack {
    let frames = frame::alloc_contiguous(SIZE / PAGE_SIZE)
        .unwrap_or_else(|| panic!("no contiguous RAM for a {SIZE:#x}-byte '{name}'"));
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

/// Snapshot pre-table stacks with the boot stack first.
pub fn all() -> impl Iterator<Item = Stack> {
    let allocated = ALLOCATED.with(|allocated| allocated.clone());
    core::iter::once(boot()).chain(allocated)
}

/// Prevent further [`alloc()`] calls before the table snapshots [`all`].
pub fn seal() { SEALED.store(true, Ordering::Release); }

pub fn report() {
    let (count, span) = ALLOCATED.with(|allocated| {
        let span = allocated.first().zip(allocated.last()).map(|(a, b)| (a.guard(), b.top()));
        (allocated.len(), span)
    });
    println!(
        "[memory] stacks: 1 boot + {count} allocated x {} (each above a {} guard)",
        ByteSize(SIZE),
        ByteSize(GUARD_SIZE)
    );
    if let Some((first, last)) = span {
        println!("[memory]   allocated stacks at {first:#x}..{last:#x}");
    }
}

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
