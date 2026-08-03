//! Port from sbi.h
//!
//! There is no `#![allow(dead_code)]` here any more. It was hiding three wrappers
//! with no callers at all, which is the same way this file accumulated the five
//! legacy IPI/RFENCE wrappers whose removal is recorded below — a blanket allow
//! makes "written but never exercised" invisible, and that is exactly the state a
//! wrapper must not be in. Functions land with their first caller.

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

// `console_getchar`, `shutdown` and `set_timer` used to sit here with zero callers.
// `set_timer`'s only user was the timer in `attic/trap/`, which is not compiled; the
// other two never had one. All three are one-line `sbi_call`s that cost nothing to
// write when something needs them, so they go back with their caller rather than
// waiting here unexercised. The attic README records that the timer brings
// `set_timer` with it.

// ---------------------------------------------------------------------------
// REMOVED: the legacy IPI and remote-fence wrappers
//
// `clear_ipi`, `send_ipi`, `remote_fence_i`, `remote_sfence_vma` and
// `remote_sfence_vma_asid` used to sit here. All five were dead — zero callers,
// invisible to the compiler behind this file's `allow(dead_code)` — and all five
// were wrong in ways that only a caller would have discovered:
//
// - Both `remote_sfence_vma*` took `start` and `size`, ignored them, and passed
//   0/0. A signature that names a range and then flushes something else is worse
//   than no function: the caller cannot tell from the call site.
// - They passed `&hart_mask as *const _ as usize`, a pointer to a stack local. The
//   v0.1 spec calls that argument a virtual address, so this is not the flat
//   misuse an earlier note here claimed — but it *is* the underspecified corner
//   that got the whole legacy interface deprecated, because implementations
//   disagreed about how a pointer handed to M-mode should be translated.
//
// The replacements are the IPI (EID 0x735049) and RFENCE (EID 0x52464E43)
// extensions, which take `hart_mask` and `hart_mask_base` BY VALUE — no pointer,
// no translation question — and carry the range arguments properly. OpenSBI
// advertises both (`ipi`, `rfnc` in its boot banner).
//
// They are not written here yet, on purpose. Nothing can call them until
// `SupervisorSoft` is handled, and there is no handler at all right now: the trap
// subsystem is parked in `crates/kernel/attic/trap/` until the boot and memory-init
// path is finalised. So anything added now would be verified by reading it. That is
// exactly how this file acquired the five functions above. They land with their
// first caller, and get tested by it.
// ---------------------------------------------------------------------------

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

// Legacy function ids. Only the one with a live wrapper is named: 0 (set_timer),
// 2 (console_getchar) and 8 (shutdown) went with the wrappers that used them, and
// 3..=7 (clear_ipi, send_ipi, the two remote fences and remote_fence_i) went with
// theirs; see the note above. That the numbering is now almost entirely gaps is the
// point — a legacy id has no reason to exist here without a caller, and filling one
// back in means using the deprecated interface again.
const SBI_CONSOLE_PUTCHAR: usize = 1;
