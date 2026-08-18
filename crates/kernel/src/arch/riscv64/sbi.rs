//! The firmware interface, in the kernel's own types.
//!
//! `sbi-rt` owns the ABI and `sbi-spec` the numbering. What this module adds is a
//! boundary: **SBI is named here and nowhere else**, so what the kernel asks of its
//! firmware is one file rather than a grep. Every call is forwarded, including the ones
//! that need no translation — a boundary that covers all of them is one a reader can rely
//! on and a grep can check.
//!
//! `sbi-rt`'s `legacy` feature stays off: v0.1 passes a hart mask by pointer and never
//! says which address space it is in.

use mmu::PhysicalAddr;
use sbi_spec::hsm::hart_state;

/// What firmware returns when it refuses a call. Re-exported so a caller can name a
/// failure without naming `sbi-spec`.
pub use sbi_spec::binary::Error;

/// Put one byte on the firmware's console.
///
/// The debug console, which takes the byte in a register: no buffer, so no physical
/// address to produce, which is what lets [`crate::console`] fall back to it when
/// producing one is the problem. Errors go nowhere, because the caller has nowhere left.
pub fn console_write_byte(byte: u8) { let _ = sbi_rt::console_write_byte(byte); }

/// A hart's state, as [`hart_get_status`] reports it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HartState {
    Started,
    Stopped,
    StartPending,
    StopPending,
    Suspended,
    SuspendPending,
    ResumePending,
    Unknown(usize),
}

impl HartState {
    fn from_raw(raw: usize) -> Self {
        match raw {
            hart_state::STARTED => Self::Started,
            hart_state::STOPPED => Self::Stopped,
            hart_state::START_PENDING => Self::StartPending,
            hart_state::STOP_PENDING => Self::StopPending,
            hart_state::SUSPENDED => Self::Suspended,
            hart_state::SUSPEND_PENDING => Self::SuspendPending,
            hart_state::RESUME_PENDING => Self::ResumePending,
            other => Self::Unknown(other),
        }
    }
}

/// Bring `hartid` up in S-mode at `entry`, with `opaque` handed to it in `a1`.
///
/// `entry` is a [`PhysicalAddr`] because the firmware starts the hart with
/// `satp = 0`: a kernel virtual address would fault on the first instruction
/// fetch. That leaves the stackless secondary entry, which installs the page
/// table and a stack before reaching Rust.
pub fn hart_start(hartid: usize, entry: PhysicalAddr, opaque: usize) -> Result<(), Error> {
    sbi_rt::hart_start(hartid, entry.bits(), opaque).into_result().map(|_| ())
}

/// Ask the firmware what state `hartid` is in.
///
/// Worth calling before [`hart_start`]: a hart may be absent, already running, or
/// reserved to the firmware, and "already started" is not the same failure as
/// "no such hart".
pub fn hart_get_status(hartid: usize) -> Result<HartState, Error> {
    sbi_rt::hart_get_status(hartid).into_result().map(HartState::from_raw)
}
