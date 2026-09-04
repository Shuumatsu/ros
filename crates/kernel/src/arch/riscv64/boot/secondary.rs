//! Secondary-hart startup.

use core::sync::atomic::{AtomicUsize, Ordering};

use mmu::{MemoryAddr, PhysicalAddr, Satp, VirtualAddr};

use super::super::sbi::{self, HartState};
use crate::memory::direct_map::virt_to_phys;

const PUBLISHED: usize = 1;

/// Release-published startup data consumed by the secondary prologue.
#[repr(C)]
pub(crate) struct SecondaryHandoff {
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
        self.ready.store(PUBLISHED, Ordering::Release);
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

    // The RISC-V ABI requires 16-byte stack alignment.
    assert!(
        stack_top.is_aligned(16),
        "hart {hartid}'s stack top {stack_top:#x} is not 16-byte aligned"
    );

    handoff.publish(satp, stack_top, cpu);

    let opaque = handoff as *const SecondaryHandoff as usize;
    sbi::hart_start(hartid, entry_address(), opaque).map_err(StartError::Rejected)
}

/// Physical secondary entry address used while firmware has `satp = 0`.
pub(crate) fn entry_address() -> PhysicalAddr {
    virt_to_phys(super::entry::secondary_entry_address())
}

const READY_OFFSET: usize = core::mem::offset_of!(SecondaryHandoff, ready);
const SATP_OFFSET: usize = core::mem::offset_of!(SecondaryHandoff, satp);
const STACK_TOP_OFFSET: usize = core::mem::offset_of!(SecondaryHandoff, stack_top);
const CPU_OFFSET: usize = core::mem::offset_of!(SecondaryHandoff, cpu);

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
        ready = const READY_OFFSET,
        satp = const SATP_OFFSET,
        stack_top = const STACK_TOP_OFFSET,
        cpu = const CPU_OFFSET,
        secondary = sym crate::start::secondary,
);
