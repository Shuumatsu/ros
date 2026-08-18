//! Secondary-hart startup: the whole protocol for bringing another hart up, and the
//! prologue that receives it.
//!
//! Together because they are two halves of one handshake — the prologue's loads are
//! `offset_of!` of the `#[repr(C)]` struct, so there is one definition and the offsets are
//! derived from it — and because [`start_cpu`] is entirely SBI HSM. Which harts to start
//! and what to give them is [`crate::cpu`]'s; *how* a hart is started is firmware's, and
//! this is where firmware is named.

use core::sync::atomic::{AtomicUsize, Ordering};

use mmu::{MemoryAddr, PhysicalAddr, Satp, VirtualAddr};

use super::super::sbi::{self, HartState};
use crate::memory::direct_map::virt_to_phys;

const PUBLISHED: usize = 1;

/// Data published by the boot hart before an SBI HSM start.
///
/// Storage belongs to the caller — one per cpu slot, so [`crate::cpu`] keeps it beside the
/// hart it describes — but every field is this module's, written by [`start_cpu`] and read
/// by the prologue below. Opaque to the owner.
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

/// Why a hart did not start.
///
/// Three cases rather than one, because they call for different reactions and
/// `hart_start`'s error code does not tell them apart: a hart the firmware reserves is a
/// machine fact, and a rejected start is a bug.
#[derive(Clone, Copy, Debug)]
pub(crate) enum StartError {
    /// Firmware reports the hart is not stopped, so there is nothing to start.
    NotStopped(HartState),
    /// Firmware would not say what state the hart is in.
    StatusUnavailable(sbi::Error),
    /// Firmware refused the start request.
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
/// The whole of the SBI HSM sequence: ask what state the hart is in, publish the handoff,
/// then request the start. Asking first because "already started" and "no such hart" are
/// different problems that `hart_start`'s error code does not distinguish.
///
/// Returning `Ok` means firmware *accepted* the request, not that the hart arrived. The
/// caller confirms arrival; nothing here can, since the hart reports in through Rust.
///
/// `handoff` must be the storage reserved for this hart and must not already have been
/// published to — it is read by the prologue with an acquire that pairs with the release
/// inside.
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

    // The RISC-V calling convention requires a 16-byte-aligned `sp`, and the prologue
    // loads this into one without adjusting it.
    assert!(
        stack_top.is_aligned(16),
        "hart {hartid}'s stack top {stack_top:#x} is not 16-byte aligned"
    );

    handoff.publish(satp, stack_top, cpu);

    let opaque = handoff as *const SecondaryHandoff as usize;
    sbi::hart_start(hartid, entry_address(), opaque).map_err(StartError::Rejected)
}

/// The address firmware is given to start a hart at, and the one the boot log reports.
///
/// Physical, and converted here rather than by the caller: firmware starts a hart with
/// `satp = 0`, so the entry it is handed has to be an address that exists before any
/// translation does. That is a fact about how this ISA's harts start, so the crossing
/// happens on this side of the boundary, and [`start_cpu`] and the log share one answer.
pub(crate) fn entry_address() -> PhysicalAddr {
    virt_to_phys(super::entry::secondary_entry_address())
}

const READY_OFFSET: usize = core::mem::offset_of!(SecondaryHandoff, ready);
const SATP_OFFSET: usize = core::mem::offset_of!(SecondaryHandoff, satp);
const STACK_TOP_OFFSET: usize = core::mem::offset_of!(SecondaryHandoff, stack_top);
const CPU_OFFSET: usize = core::mem::offset_of!(SecondaryHandoff, cpu);

boot_fn!(
    /// Adopt the kernel page table, the stack the boot hart reserved and the [`Cpu`]
    /// it chose, then enter Rust.
    ///
    /// Still assembly, because the stack is only mapped by the kernel table: `sp` cannot be
    /// set before the switch, and no Rust runs before `sp`.
    ///
    /// Reached from `super::entry::enter_high` with `a0` the hart id and `a1` the `opaque`
    /// from `hart_start` — this hart's handoff, as a kernel VA, already reachable because
    /// the boot table maps the high half too.
    ///
    /// [`Cpu`]: crate::cpu::Cpu
    pub(super) fn prologue in entry {
        // Does not spin in practice — `publish` finishes before `hart_start` — but it is
        // the acquire half of that release store: SBI does not promise the start request
        // orders the boot hart's writes, and every field below is garbage if nothing does.
        "1:",
        "ld    t0, {ready}(a1)",
        "beqz  t0, 1b",
        "fence r, rw",
        // Read out while `a1` is still the handoff: it is about to become the argument.
        "ld    t0, {satp}(a1)",
        "ld    t1, {stack_top}(a1)",
        "ld    a1, {cpu}(a1)",
        "csrw  satp, t0",
        "sfence.vma",
        // Before the `tail`, which expands through `t1`.
        "mv    sp, t1",
        // The outermost frame on this stack, so zero is what stops a gdb unwind.
        "mv    ra, zero",
        "tail  {secondary}",
    }
        ready = const READY_OFFSET,
        satp = const SATP_OFFSET,
        stack_top = const STACK_TOP_OFFSET,
        cpu = const CPU_OFFSET,
        secondary = sym crate::start::secondary,
);
