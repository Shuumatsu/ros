//! Switching the hart between two kernel contexts.
//!
//! Contexts preserve the ABI's callee-saved registers, `ra`, and `sp`. `gp` is image-wide,
//! while `tp` remains the identity of the hart executing the context.

use core::mem::offset_of;

use mmu::{MemoryAddr, VirtualAddr};

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
    /// # Panics
    ///
    /// Panics if `stack_top` is not 16-byte aligned as required by the RISC-V ABI.
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
        /// Returns only after another context switches back into `save`.
        ///
        /// # Safety
        ///
        /// Both pointers must be valid. `load` must have been built by [`KernelContext::new`] or
        /// saved by this function, and its stack must remain mapped and owned by this hart.
        #[unsafe(naked)]
        pub unsafe extern "C" fn switch(save: *mut KernelContext, load: *const KernelContext) {
            ::core::arch::naked_asm!(
                $(concat!("sd ", stringify!($reg), ", {", stringify!($reg), "}(a0)"),)*
                $(concat!("ld ", stringify!($reg), ", {", stringify!($reg), "}(a1)"),)*
                "ret",
                $($reg = const offset_of!(KernelContext, $reg),)*
            )
        }
    };
}
saved_registers!(define_switch);

/// Trampoline for a context built by [`KernelContext::new`].
#[unsafe(naked)]
unsafe extern "custom" fn enter() {
    ::core::arch::naked_asm!("mv a0, s1", "jr s0");
}

struct Handoff {
    resume: KernelContext,
    observed: usize,
}

/// Switches to `stack_top` and returns the stack pointer observed there.
pub fn self_test(stack_top: VirtualAddr) -> VirtualAddr {
    let mut handoff = Handoff { resume: KernelContext::default(), observed: 0 };
    let there = KernelContext::new(report_sp, stack_top, &raw mut handoff as usize);

    // SAFETY: `there` names `stack_top`, which the caller has mapped on this hart, and
    // `report_sp` switches straight back into the context saved here.
    unsafe { switch(&raw mut handoff.resume, &raw const there) };

    VirtualAddr::new(handoff.observed)
}

extern "C" fn report_sp(handoff: usize) -> ! {
    let handoff = handoff as *mut Handoff;
    let sp = super::sp().bits();

    // SAFETY: `handoff` remains live on the suspended caller's stack and is exclusively accessed.
    unsafe {
        (*handoff).observed = sp;
        let mut spent = KernelContext::default();
        switch(&raw mut spent, &raw const (*handoff).resume);
    }

    unreachable!("switched back into a kernel context that had already returned")
}
