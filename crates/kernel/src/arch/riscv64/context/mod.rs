//! Switching the hart between two kernel contexts.
//!
//! Contexts preserve the ABI's callee-saved registers, `ra`, and `sp`. `gp` is image-wide,
//! while `tp` remains the identity of the hart executing the context.

mod self_test;

use core::mem::offset_of;

use mmu::{MemoryAddr, VirtualAddr};

use super::{STACK_ALIGN, address_of};

pub use self_test::run as self_test;

macro_rules! switched_registers {
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
switched_registers!(define_context);

impl KernelContext {
    /// A context that will begin at `entry(arg)` on `stack_top`.
    ///
    /// # Panics
    ///
    /// Panics if `stack_top` is not aligned as the ABI requires of `sp`.
    pub fn new(entry: extern "C" fn(usize) -> !, stack_top: VirtualAddr, arg: usize) -> Self {
        assert!(
            stack_top.is_aligned(STACK_ALIGN),
            "kernel stack top {stack_top:#x} is not {STACK_ALIGN}-byte aligned"
        );
        Self {
            ra: address_of(enter).bits(),
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

        /// Abandon the calling context and resume `load`, which never switches back.
        ///
        /// # Safety
        ///
        /// `load` must meet [`switch`]'s contract, and nothing may still depend on the calling
        /// context's registers or stack.
        #[unsafe(naked)]
        pub unsafe extern "C" fn switch_to(load: *const KernelContext) -> ! {
            ::core::arch::naked_asm!(
                $(concat!("ld ", stringify!($reg), ", {", stringify!($reg), "}(a0)"),)*
                "ret",
                $($reg = const offset_of!(KernelContext, $reg),)*
            )
        }
    };
}
switched_registers!(define_switch);

/// Trampoline for a context built by [`KernelContext::new`].
#[unsafe(naked)]
unsafe extern "custom" fn enter() {
    ::core::arch::naked_asm!("mv a0, s1", "jr s0");
}
