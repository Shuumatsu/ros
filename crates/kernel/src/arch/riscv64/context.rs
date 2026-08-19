//! Switching the hart between two kernel contexts.
//!
//! A *context* here is what one thread of kernel control needs to be resumed: the callee-saved
//! registers, the return address, and the stack pointer. Everything else is either caller-saved —
//! the compiler already spilled it — or not per-context at all.
//!
//! `tp` and `gp` are deliberately absent. `gp` is one value for the whole image, and `tp` is the
//! *hart's* identity rather than the context's: restoring a saved `tp` would let a context resumed
//! on a second hart go on claiming the first hart's control block.
//!
//! Mechanism with no policy. Which context runs next is a decision nothing here makes, and no
//! queue, priority or time slice appears below — [`switch`] is the same primitive whether it is
//! called once at boot or by a scheduler.

use core::mem::offset_of;

use mmu::{MemoryAddr, VirtualAddr};

/// The registers a context switch preserves, in ABI order.
///
/// `sp` is an ordinary member: a context is reached through a pointer in an argument register, so
/// nothing about saving the stack pointer here is circular.
macro_rules! saved_registers {
    ($target:ident) => {
        $target! {
            ra, sp,
            s0, s1, s2, s3, s4, s5, s6, s7, s8, s9, s10, s11
        }
    };
}

macro_rules! define_context {
    ($($reg:ident),*) => {
        /// A suspended kernel context. `Default` is the empty one a first [`switch`] saves into.
        #[repr(C)]
        #[derive(Clone, Copy, Default)]
        pub struct KernelContext {
            $($reg: usize,)*
        }
    };
}
saved_registers!(define_context);

impl KernelContext {
    /// A context that will begin at `entry(arg)` on `stack_top`.
    ///
    /// `entry` never returns: it is reached by a jump rather than a call, so there is no return
    /// address beneath it. A context that means to hand the hart back calls [`switch`].
    ///
    /// # Panics
    ///
    /// If `stack_top` is not 16-byte aligned, which the RISC-V ABI requires of `sp` at all times.
    pub fn new(entry: extern "C" fn(usize) -> !, stack_top: VirtualAddr, arg: usize) -> Self {
        assert!(stack_top.is_aligned(16), "kernel stack top {stack_top:#x} is not 16-byte aligned");
        Self {
            ra: enter as *const () as usize,
            sp: stack_top.bits(),
            s0: entry as usize,
            s1: arg,
            ..Self::default()
        }
    }
}

macro_rules! define_switch {
    ($($reg:ident),*) => {
        /// Suspend the calling context into `save`, and resume `load`.
        ///
        /// Returns when something switches back into `save`: same stack, same frame, every
        /// callee-saved register as it was. A context nothing resumes never returns from here.
        ///
        /// Raw pointers rather than references, because another thread of kernel control runs
        /// while this call is in progress and will reach the same two contexts. There is no
        /// moment at which one Rust borrow describes them.
        ///
        /// # Safety
        ///
        /// Both pointers must be valid and `load` must be a context [`KernelContext::new`] built
        /// or a previous call here saved, on a stack mapped for the calling hart. Resuming a
        /// context whose stack belongs to another hart, or whose frames are gone, hands the hart a
        /// stack pointer into memory nothing owns.
        #[unsafe(naked)]
        pub unsafe extern "C" fn switch(save: *mut KernelContext, load: *const KernelContext) {
            ::core::arch::naked_asm!(
                $(concat!("sd ", stringify!($reg), ", {", stringify!($reg), "}(a0)"),)*
                $(concat!("ld ", stringify!($reg), ", {", stringify!($reg), "}(a1)"),)*
                // `ra` belongs to `load` now, so this returns into whoever saved it.
                "ret",
                $($reg = const offset_of!(KernelContext, $reg),)*
            )
        }
    };
}
saved_registers!(define_switch);

/// Where a context built by [`KernelContext::new`] begins.
///
/// The first [`switch`] into such a context loads `s0` and `s1` with the entry and its argument
/// and `ra` with this, so returning from that switch lands here, on the new stack, holding both.
#[unsafe(naked)]
unsafe extern "custom" fn enter() {
    ::core::arch::naked_asm!("mv a0, s1", "jr s0");
}

/// What [`self_test`] leaves on its own stack for the context it starts.
struct Handoff {
    /// The context to resume to get back here.
    resume: KernelContext,
    /// Stack pointer observed on the other stack, `0` until it reports one.
    observed: usize,
}

/// Switch onto `stack_top` and back, answering with the stack pointer observed over there.
///
/// The proof has to come from the other stack, which is why the answer is an `sp`: a caller that
/// knows the stack's extent can check the value lies inside it, and a switch that never arrived
/// cannot produce one at all.
pub fn self_test(stack_top: VirtualAddr) -> VirtualAddr {
    let mut handoff = Handoff { resume: KernelContext::default(), observed: 0 };
    let there = KernelContext::new(report_sp, stack_top, &raw mut handoff as usize);

    // SAFETY: `there` names `stack_top`, which the caller has mapped on this hart, and
    // `report_sp` switches straight back into the context saved here.
    unsafe { switch(&raw mut handoff.resume, &raw const there) };

    VirtualAddr::new(handoff.observed)
}

/// Report this context's stack pointer through the handoff, then give the hart back.
extern "C" fn report_sp(handoff: usize) -> ! {
    let handoff = handoff as *mut Handoff;
    let sp = super::sp().bits();

    // SAFETY: `handoff` is the `Handoff` `self_test` left on its own stack, which is live and
    // suspended inside `switch`, so this is the only context touching it.
    unsafe {
        (*handoff).observed = sp;
        // Saved into a context on this stack that nothing will resume: the switch is one-way.
        let mut spent = KernelContext::default();
        switch(&raw mut spent, &raw const (*handoff).resume);
    }

    unreachable!("switched back into a kernel context that had already returned")
}
