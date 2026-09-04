//! Secondary-hart startup.

use core::mem::offset_of;
use core::sync::atomic::{AtomicUsize, Ordering};

use mmu::{MemoryAddr, PhysicalAddr, Satp, VirtualAddr};

use super::super::sbi::{self, HartState};
use super::super::{STACK_ALIGN, address_of};
use crate::memory::direct_map::virt_to_phys;

/// Release-published startup data consumed by the secondary prologue.
#[repr(C)]
pub(crate) struct SecondaryHandoff {
    /// Zero until the fields below are complete; the prologue spins on it.
    ready: AtomicUsize,
    satp: AtomicUsize,
    stack_top: AtomicUsize,
    cpu: AtomicUsize,
}

impl SecondaryHandoff {
    pub(crate) const fn new() -> Self {
        Self {
            ready: AtomicUsize::new(0),
            satp: AtomicUsize::new(0),
            stack_top: AtomicUsize::new(0),
            cpu: AtomicUsize::new(0),
        }
    }

    fn publish(&self, satp: Satp, stack_top: VirtualAddr, cpu: usize) {
        assert_eq!(self.ready.load(Ordering::Relaxed), 0, "secondary handoff published twice");
        assert_ne!(satp.bits(), 0, "secondary handoff needs a live page table");
        assert_ne!(stack_top.bits(), 0, "secondary handoff needs a stack");
        assert_ne!(cpu, 0, "secondary handoff needs a Cpu");

        self.satp.store(satp.bits(), Ordering::Relaxed);
        self.stack_top.store(stack_top.bits(), Ordering::Relaxed);
        self.cpu.store(cpu, Ordering::Relaxed);
        self.ready.store(1, Ordering::Release);
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum StartError {
    NotStopped(HartState),
    StatusUnavailable(sbi::Error),
    Rejected(sbi::Error),
}

impl core::fmt::Display for StartError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotStopped(state) => write!(f, "firmware reports {state:?}"),
            Self::StatusUnavailable(error) => write!(f, "status unavailable: {error:?}"),
            Self::Rejected(error) => write!(f, "firmware refused the start: {error:?}"),
        }
    }
}

/// Start `hartid` on `satp` with `stack_top`, handing it `cpu` to adopt as its own.
///
/// `Ok` means firmware accepted the request; the caller must confirm arrival. `handoff`
/// must be reserved for this hart and unpublished.
pub(crate) fn start_cpu(
    hartid: usize,
    handoff: &SecondaryHandoff,
    satp: Satp,
    stack_top: VirtualAddr,
    cpu: usize,
) -> Result<(), StartError> {
    match sbi::hart_get_status(hartid) {
        Ok(HartState::Stopped) => {}
        Ok(state) => return Err(StartError::NotStopped(state)),
        Err(error) => return Err(StartError::StatusUnavailable(error)),
    }

    assert!(
        stack_top.is_aligned(STACK_ALIGN),
        "hart {hartid}'s stack top {stack_top:#x} is not {STACK_ALIGN}-byte aligned"
    );

    handoff.publish(satp, stack_top, cpu);

    let opaque = handoff as *const SecondaryHandoff as usize;
    sbi::hart_start(hartid, entry_address(), opaque).map_err(StartError::Rejected)
}

/// Physical secondary entry address used while firmware has `satp = 0`.
pub(crate) fn entry_address() -> PhysicalAddr {
    virt_to_phys(address_of(super::entry::secondary_entry))
}

boot_fn!(
    /// Installs the published page table, stack, and CPU pointer before entering Rust.
    pub(super) fn prologue in entry {
        // This acquire pairs with `publish` because SBI does not order the boot hart's writes.
        "1:",
        "ld    t0, {ready}(a1)",
        "beqz  t0, 1b",
        "fence r, rw",
        "ld    t0, {satp}(a1)",
        "ld    t1, {stack_top}(a1)",
        "ld    a1, {cpu}(a1)",
        "csrw  satp, t0",
        "sfence.vma",
        "mv    sp, t1",
        // Terminate stack unwinding at this outermost frame.
        "mv    ra, zero",
        "tail  {secondary}",
    }
        ready = const offset_of!(SecondaryHandoff, ready),
        satp = const offset_of!(SecondaryHandoff, satp),
        stack_top = const offset_of!(SecondaryHandoff, stack_top),
        cpu = const offset_of!(SecondaryHandoff, cpu),
        secondary = sym crate::start::secondary,
);
