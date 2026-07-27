//! Port from sbi.h
#![allow(dead_code)]

use core::arch::asm;

#[inline(always)]
fn sbi_call(which: usize, arg0: usize, arg1: usize, arg2: usize) -> usize {
    let ret;
    unsafe {
        asm!(
            "ecall",
            in("a0") arg0,
            in("a1") arg1,
            in("a2") arg2,
            in("a7") which,
            lateout("a0") ret,
            options(nostack)
        );
    }
    ret
}

pub fn console_putchar(ch: usize) { sbi_call(SBI_CONSOLE_PUTCHAR, ch, 0, 0); }

pub fn console_getchar() -> usize { sbi_call(SBI_CONSOLE_GETCHAR, 0, 0, 0) }

pub fn shutdown() -> ! {
    sbi_call(SBI_SHUTDOWN, 0, 0, 0);
    unreachable!()
}

pub fn set_timer(stime_value: u64) { sbi_call(SBI_SET_TIMER, stime_value as usize, 0, 0); }

pub fn clear_ipi() { sbi_call(SBI_CLEAR_IPI, 0, 0, 0); }

pub fn send_ipi(hart_mask: usize) { sbi_call(SBI_SEND_IPI, &hart_mask as *const _ as usize, 0, 0); }

pub fn remote_fence_i(hart_mask: usize) {
    sbi_call(SBI_REMOTE_FENCE_I, &hart_mask as *const _ as usize, 0, 0);
}

pub fn remote_sfence_vma(hart_mask: usize, _start: usize, _size: usize) {
    sbi_call(SBI_REMOTE_SFENCE_VMA, &hart_mask as *const _ as usize, 0, 0);
}

pub fn remote_sfence_vma_asid(hart_mask: usize, _start: usize, _size: usize, _asid: usize) {
    sbi_call(SBI_REMOTE_SFENCE_VMA_ASID, &hart_mask as *const _ as usize, 0, 0);
}

// ===========================================================================
// SBI v0.2+ extensions
//
// The legacy calls above pass a function number in `a7` and return a single
// value in `a0`. Everything since v0.2 splits that: `a7` selects an *extension*
// and `a6` a function within it, and the return is a pair — error in `a0`, value
// in `a1`. The two forms cannot share `sbi_call`, hence the second one below.
// ===========================================================================

/// An SBI call that failed, carrying the spec's negative error code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SbiError(pub isize);

impl core::fmt::Display for SbiError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let text = match self.0 {
            -1 => "failed",
            -2 => "not supported",
            -3 => "invalid parameter",
            -4 => "denied",
            -5 => "invalid address",
            -6 => "already available",
            -7 => "already started",
            -8 => "already stopped",
            _ => "unknown error",
        };
        write!(f, "{text} ({})", self.0)
    }
}

/// Make an SBI v0.2+ call: extension `eid`, function `fid`.
#[inline]
fn sbi_call_ext(eid: usize, fid: usize, arg0: usize, arg1: usize, arg2: usize) -> Result<usize, SbiError> {
    let (error, value): (isize, usize);
    unsafe {
        asm!(
            "ecall",
            inlateout("a0") arg0 => error,
            inlateout("a1") arg1 => value,
            in("a2") arg2,
            in("a6") fid,
            in("a7") eid,
            options(nostack)
        );
    }
    if error == 0 { Ok(value) } else { Err(SbiError(error)) }
}

/// Hart State Management extension, `"HSM"` in ASCII.
const SBI_EXT_HSM: usize = 0x48534D;
const HSM_HART_START: usize = 0;
const HSM_HART_GET_STATUS: usize = 2;

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
            0 => Self::Started,
            1 => Self::Stopped,
            2 => Self::StartPending,
            3 => Self::StopPending,
            4 => Self::Suspended,
            5 => Self::SuspendPending,
            6 => Self::ResumePending,
            other => Self::Unknown(other),
        }
    }
}

/// Bring `hartid` up in S-mode at `start_addr`, with `opaque` handed to it in `a1`.
///
/// # `start_addr` must be PHYSICAL
///
/// The spec starts the target hart with `satp = 0` — translation off. So this cannot
/// point at a Rust function: those are linked at high virtual addresses, and fetching
/// the first instruction would fault before any page table exists. It has to be the
/// physical address of `_start`, and the hart then walks the same `boot.S` path the
/// boot hart did: install the early table, jump high, then enter Rust.
pub fn hart_start(hartid: usize, start_addr: usize, opaque: usize) -> Result<(), SbiError> {
    sbi_call_ext(SBI_EXT_HSM, HSM_HART_START, hartid, start_addr, opaque).map(|_| ())
}

/// Ask the firmware what state `hartid` is in.
///
/// Worth calling before [`hart_start`]: a hart may be absent, already running, or
/// reserved to the firmware, and "already started" is not the same failure as
/// "no such hart".
pub fn hart_get_status(hartid: usize) -> Result<HartState, SbiError> {
    sbi_call_ext(SBI_EXT_HSM, HSM_HART_GET_STATUS, hartid, 0, 0).map(HartState::from_raw)
}

const SBI_SET_TIMER: usize = 0;
const SBI_CONSOLE_PUTCHAR: usize = 1;
const SBI_CONSOLE_GETCHAR: usize = 2;
const SBI_CLEAR_IPI: usize = 3;
const SBI_SEND_IPI: usize = 4;
const SBI_REMOTE_FENCE_I: usize = 5;
const SBI_REMOTE_SFENCE_VMA: usize = 6;
const SBI_REMOTE_SFENCE_VMA_ASID: usize = 7;
const SBI_SHUTDOWN: usize = 8;
