//! An address space: a root table, the tree under it, and the `satp` that names it.
//!
//! Owning the root and handing out a [`Mapper`] is what keeps a table from being
//! write-once; without it the next subsystem needing a mapping has no way in but a second
//! `&mut` to the same root. Layout is not this module's — [`super::kernel_table`] is the
//! kernel's instance, a user space would be another.
//!
//! Reaching the tree goes through [`AddressSpace::edit`] or [`AddressSpace::walk`], which is
//! what pairs every write with a TLB fence. A mapper handed out bare would let a caller
//! install leaves the hardware never looks at.

use mmu::{FrameSource, LinearOffset, Mapper, PhysicalAddr, Satp, Scheme, Table};

use super::direct_map::{VA_OFFSET, phys_to_virt};
use super::{KernelScheme, frame};
use crate::arch::tlb;

/// The kernel's one mapper flavour, binding the three choices [`mmu`] leaves open: the
/// translation scheme, where intermediate tables come from, and how a frame is reached.
pub type KernelMapper<'a> = Mapper<'a, KernelScheme, TableFrames, LinearOffset>;

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

/// A live page-table tree and the register value that installs it.
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
        // The mode is the scheme's, so a tree and the register that installs it cannot
        // disagree about how deep the walk is.
        Self { satp: Satp::new(KernelScheme::MODE, asid, root_pa), root }
    }

    /// Physical address of the root table.
    pub fn root(&self) -> PhysicalAddr { self.satp.root() }

    /// The `satp` value that makes this space live.
    pub fn satp(&self) -> Satp { self.satp }

    /// Change this space's mappings, then retire the translations the change invalidated.
    ///
    /// The only way to reach a `&mut` mapper, which is what makes the fence unskippable: a
    /// hart may have cached the *absence* of a translation, so a leaf installed without one
    /// is a mapping the hardware ignores. `&mut self`, so edits are serialised by whoever
    /// owns the space.
    ///
    /// Fenced unconditionally rather than only when this space is live, because "is any hart
    /// running this tree" is not a question the calling hart can answer — and for the same
    /// reason this is not enough for a tree live on *another* hart. See [`tlb`].
    pub fn edit<R>(&mut self, f: impl FnOnce(&mut KernelMapper<'_>) -> R) -> R {
        let result = f(&mut self.mapper());
        tlb::flush_all();
        result
    }

    /// Walk this space's mappings without changing them.
    ///
    /// The mapper arrives as `&`, and every operation that writes a table takes `&mut self`,
    /// so a walk cannot invalidate a translation and needs no fence. That is a type-level
    /// distinction, not a promise in a doc comment.
    pub fn walk<R>(&mut self, f: impl FnOnce(&KernelMapper<'_>) -> R) -> R { f(&self.mapper()) }

    /// The tree, bound to the kernel's frame source and addressing policy.
    ///
    /// Private: handed out only through [`edit`](Self::edit) and [`walk`](Self::walk), so
    /// there is no way to obtain a mutable mapper and skip the fence.
    fn mapper(&mut self) -> KernelMapper<'_> {
        Mapper::new(&mut *self.root, TableFrames, LinearOffset(VA_OFFSET))
    }

    /// Make this space the live translation on the calling hart.
    ///
    /// # Safety
    ///
    /// This tree must map the calling hart's running PC and stack pointer to the same
    /// physical addresses the live tree does, or this faults on the next instruction with
    /// no table left to diagnose it from. [`super::kernel_table`] audits that first.
    pub unsafe fn activate(&self) {
        // SAFETY: forwarded from this function's contract.
        unsafe { tlb::install(self.satp) };
    }
}
