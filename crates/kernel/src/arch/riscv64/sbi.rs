//! Supervisor Binary Interface wrappers.

use mmu::PhysicalAddr;
use sbi_spec::hsm::hart_state;

pub use sbi_spec::binary::Error;

/// Writes one byte to the debug console and ignores firmware errors.
pub fn console_write_byte(byte: u8) { let _ = sbi_rt::console_write_byte(byte); }

/// Arms the supervisor timer and clears any pending timer interrupt.
pub fn set_timer(deadline: u64) -> Result<(), Error> {
    sbi_rt::set_timer(deadline).into_result().map(|_| ())
}

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
/// Firmware starts the hart with `satp = 0`, so `entry` must be physical.
pub fn hart_start(hartid: usize, entry: PhysicalAddr, opaque: usize) -> Result<(), Error> {
    sbi_rt::hart_start(hartid, entry.bits(), opaque).into_result().map(|_| ())
}

pub fn hart_get_status(hartid: usize) -> Result<HartState, Error> {
    sbi_rt::hart_get_status(hartid).into_result().map(HartState::from_raw)
}
