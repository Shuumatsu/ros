//! Page-table tree ownership and activation.
//!
//! Mapping edits through [`AddressSpace::edit`] flush the calling hart's TLB.

use mmu::{
    Entry, FrameSource, LinearOffset, Mapper, PhysicalAddr, Satp, Scheme, Table, VirtualAddr,
};

use super::direct_map::{VA_OFFSET, phys_to_virt};
use super::{KernelScheme, frame};
use crate::arch::tlb;

pub type KernelMapper<'a> = Mapper<'a, KernelScheme, TableFrames, LinearOffset>;

/// Supplies frames whose branch PTEs become their ownership records.
pub struct TableFrames;

// SAFETY: allocated frames are aligned, zeroed, and exclusive until `free_at`.
unsafe impl FrameSource for TableFrames {
    fn alloc_zeroed(&mut self) -> Option<PhysicalAddr> { frame::alloc().map(frame::Frames::leak) }

    unsafe fn free(&mut self, frame: PhysicalAddr) {
        // SAFETY: the trait contract returns a singly allocated frame from this source.
        unsafe { frame::free_at(frame) };
    }
}

/// A page-table tree and its `satp` value.
pub struct AddressSpace {
    root: &'static mut Table,
    satp: Satp,
}

impl AddressSpace {
    /// Build an empty address space.
    ///
    /// The root frame is retained because destruction cannot ensure it is absent from every hart.
    ///
    /// # Panics
    ///
    /// Panics if no root frame is available.
    pub fn new(asid: usize) -> Self {
        let root_pa = frame::alloc().expect("no frame for a page-table root").leak();
        // SAFETY: this aligned, zeroed frame is exclusively owned and permanently retained.
        let root: &'static mut Table = unsafe { &mut *phys_to_virt(root_pa).as_mut_ptr::<Table>() };
        Self { satp: Satp::new(KernelScheme::MODE, asid, root_pa), root }
    }

    pub fn root(&self) -> PhysicalAddr { self.satp.root() }

    pub fn satp(&self) -> Satp { self.satp }

    /// Point this space's upper half at the same subtrees as `other`'s.
    ///
    /// Root slots added to `other` afterwards are invisible here. The new space must not be live.
    pub fn share_upper_half_from(&mut self, other: &AddressSpace) {
        self.root.share_upper_half(other.root);
    }

    pub fn root_slot(&self, vaddr: VirtualAddr) -> Entry {
        self.root.root_slot::<KernelScheme>(vaddr)
    }

    /// Edit mappings and flush the calling hart's TLB, including cached missing translations.
    ///
    /// Other harts running this tree require a separate shootdown.
    pub fn edit<R>(&mut self, f: impl FnOnce(&mut KernelMapper<'_>) -> R) -> R {
        let result = f(&mut self.mapper());
        tlb::flush_all();
        result
    }

    pub fn walk<R>(&mut self, f: impl FnOnce(&KernelMapper<'_>) -> R) -> R { f(&self.mapper()) }

    fn mapper(&mut self) -> KernelMapper<'_> {
        Mapper::new(&mut *self.root, TableFrames, LinearOffset(VA_OFFSET))
    }

    /// Make this space the live translation on the calling hart.
    ///
    /// # Safety
    ///
    /// The tree must preserve the calling hart's current PC and SP physical mappings.
    pub unsafe fn activate(&self) {
        // SAFETY: forwarded from this function's contract.
        unsafe { tlb::install(self.satp) };
    }
}
