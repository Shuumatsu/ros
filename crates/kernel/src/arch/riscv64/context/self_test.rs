//! Proof that a kernel context runs on a given stack and switches back.

use mmu::VirtualAddr;

use super::{KernelContext, switch, switch_to};

struct Handoff {
    resume: KernelContext,
    observed: usize,
}

/// Switches to `stack_top` and returns the stack pointer observed there.
pub fn run(stack_top: VirtualAddr) -> VirtualAddr {
    let mut handoff = Handoff { resume: KernelContext::default(), observed: 0 };
    let there = KernelContext::new(report_sp, stack_top, &raw mut handoff as usize);

    // SAFETY: `there` names `stack_top`, which the caller has mapped on this hart, and
    // `report_sp` switches straight back into the context saved here.
    unsafe { switch(&raw mut handoff.resume, &raw const there) };

    VirtualAddr::new(handoff.observed)
}

extern "C" fn report_sp(handoff: usize) -> ! {
    let handoff = handoff as *mut Handoff;
    let sp = super::super::sp().bits();

    // SAFETY: `handoff` remains live on the suspended caller's stack and is exclusively accessed.
    unsafe {
        (*handoff).observed = sp;
        switch_to(&raw const (*handoff).resume)
    }
}
