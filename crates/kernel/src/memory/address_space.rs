//! An Sv39 address space: a root table, the tree under it, and the `satp` that names it.
//!
//! Owning the root and handing out a [`Mapper`] is what keeps a table from being
//! write-once; without it the next subsystem needing a mapping has no way in but a second
//! `&mut` to the same root. Layout is not this module's — [`super::kernel_table`] is the
//! kernel's instance, a user space would be another.

use paging::sv39::FrameSource;
use paging::{LinearOffset, Mapper, PhysicalAddr, Satp, Table};

use crate::memory::direct_map::VA_OFFSET;
use crate::memory::{frame, phys_to_virt};

/// The kernel's one mapper flavour, binding the two policies [`paging`] leaves open.
pub type KernelMapper<'a> = Mapper<'a, TableFrames, LinearOffset>;

/// Supplies the frames intermediate page tables live in.
///
/// Dropping the [`frame::Frames`] token is the handoff, not a leak: once the frame is a
/// branch PTE, the page table is its record of ownership, which is what
/// [`frame::free_at`] exists for.
pub struct TableFrames;

// SAFETY: `frame::alloc` returns a page-aligned, freshly zeroed frame — which is what
// makes a new table read as "all entries invalid" — owned exclusively until `free_at`.
unsafe impl FrameSource for TableFrames {
    fn alloc_zeroed(&mut self) -> Option<PhysicalAddr> { frame::alloc().map(frame::Frames::leak) }

    unsafe fn free(&mut self, frame: PhysicalAddr) {
        // SAFETY: forwarded from the trait's contract; `alloc_zeroed` vends single
        // frames, which is `free_at`'s order-0 requirement.
        unsafe { frame::free_at(frame) };
    }
}

/// A live Sv39 tree and the register value that installs it.
pub struct AddressSpace {
    root: &'static mut Table,
    /// Also the only record of the root's physical address; a copy alongside would be a
    /// second answer.
    satp: Satp,
}

impl AddressSpace {
    /// Build an empty address space: a fresh root table, nothing mapped.
    ///
    /// The root frame is leaked rather than dropped — tearing a space down means clearing
    /// it out of every hart's `satp` first, which no destructor can arrange.
    ///
    /// # Panics
    ///
    /// If the frame allocator cannot produce a frame for the root.
    pub fn new(asid: usize) -> Self {
        let root_pa = frame::alloc().expect("no frame for a page-table root").leak();
        // SAFETY: a zeroed, page-aligned frame this value now owns exclusively and never
        // releases, reachable through the direct map, so the `'static mut` is unique.
        let root: &'static mut Table = unsafe { &mut *phys_to_virt(root_pa).as_mut_ptr::<Table>() };
        Self { satp: Satp::sv39(root_pa, asid), root }
    }

    /// Physical address of the root table.
    pub fn root(&self) -> PhysicalAddr { self.satp.root() }

    /// The `satp` value that makes this space live.
    pub fn satp(&self) -> Satp { self.satp }

    /// Borrow this space for mapping, unmapping or walking — the one way to reach the
    /// tree, and `&mut`, so edits are serialised by whoever owns the space.
    pub fn mapper(&mut self) -> KernelMapper<'_> {
        Mapper::new(&mut *self.root, TableFrames, LinearOffset(VA_OFFSET))
    }

    /// Make this space the live translation on the calling hart, and flush the TLB.
    ///
    /// Interrupts are masked across the pair so no trap can observe a half-switched
    /// translation.
    ///
    /// # Safety
    ///
    /// This tree must map the calling hart's running PC and stack pointer to the same
    /// physical addresses the live tree does, or this faults on the next instruction with
    /// no table left to diagnose it from. [`super::kernel_table`] audits that first.
    pub unsafe fn activate(&self) {
        let bits = self.satp.bits();
        crate::arch::riscv64::interrupts::without(|| {
            // SAFETY: forwarded from this function's contract.
            unsafe {
                core::arch::asm!(
                    "csrw satp, {satp}",
                    "sfence.vma",
                    satp = in(reg) bits,
                    options(nostack)
                );
            }
        });
    }
}
