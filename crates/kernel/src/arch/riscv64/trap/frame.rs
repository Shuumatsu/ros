//! What a trap saves, and the assembly that saves it.
//!
//! One frame, not one per privilege level. A supervisor trap and a future return to user
//! mode both mean "the register file as the interrupted code left it", and two types
//! spelling that would be two answers to one question. So every general register is here,
//! not only the caller-saved ones a Rust handler would otherwise clobber: the extra stores
//! are what let a context switch and a fault dump read the same frame.
//!
//! The register list is stated once, in [`saved_registers`]. The frame's fields, the stores
//! that fill it, the loads that undo them and the dump a fault prints are all expansions of
//! that one list, so a slot cannot disagree with the instruction that writes it.

use core::fmt;
use core::mem::offset_of;

use mmu::VirtualAddr;

/// The general registers a trap saves, in x-register order.
///
/// `sp` is absent, and is the one field [`TrapFrame`] declares by hand: no instruction
/// naming it can store it, because moving `sp` is what created the frame it would be stored
/// into. The entry stores it from a scratch register instead.
macro_rules! saved_registers {
    ($target:ident) => {
        $target! {
            ra, gp, tp,
            t0, t1, t2,
            s0, s1,
            a0, a1, a2, a3, a4, a5, a6, a7,
            s2, s3, s4, s5, s6, s7, s8, s9, s10, s11,
            t3, t4, t5, t6
        }
    };
}

macro_rules! define_frame {
    ($($reg:ident),*) => {
        /// The interrupted context: the two CSRs `sret` consumes, then the register file.
        ///
        /// `align(16)` rather than a padding field: the entry makes room by subtracting
        /// this type's size from `sp`, and the RISC-V ABI keeps `sp` 16-byte aligned at all
        /// times, so the alignment is what rounds the size up and nothing has to name the
        /// slack.
        #[repr(C, align(16))]
        #[derive(Clone, Copy)]
        pub struct TrapFrame {
            pub sepc: usize,
            pub sstatus: usize,
            pub sp: usize,
            $(pub $reg: usize,)*
        }

        /// The register file, four to a line, `\r\n` because a serial console reads it.
        impl fmt::Display for TrapFrame {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "  sepc {:#018x}  sstatus {:#018x}", self.sepc, self.sstatus)?;
                write!(f, "\r\n  {:>3} {:#018x}", "sp", self.sp)?;

                let mut written = 1;
                $(
                    f.write_str(if written % 4 == 0 { "\r\n  " } else { "  " })?;
                    write!(f, "{:>3} {:#018x}", stringify!($reg), self.$reg)?;
                    written += 1;
                )*
                let _ = written;
                Ok(())
            }
        }
    };
}
saved_registers!(define_frame);

macro_rules! define_entry {
    ($($reg:ident),*) => {
        /// Where every supervisor trap lands: save the interrupted context, dispatch it in
        /// Rust, put it back, `sret`.
        ///
        /// Runs on the interrupted stack. That holds for as long as every trap comes from
        /// the kernel — a trap from user mode arrives on a stack the kernel must not push
        /// onto, which is what `sscratch` is for and why this writes it nowhere yet.
        ///
        /// Naked and `extern "custom"`: the hardware enters it with no arguments and it
        /// leaves through `sret`, so there is no Rust ABI to honour on either side.
        #[unsafe(naked)]
        unsafe extern "custom" fn trap_entry() {
            ::core::arch::naked_asm!(
                // `stvec` reads the low two bits of its address as a mode, so the vector
                // must be 4-byte aligned. rustc gives each function its own section, so
                // this raises *this* function's alignment; `install` asserts the result
                // rather than trusting the toolchain to keep doing that.
                ".balign 4",

                "addi sp, sp, -{frame_size}",
                $(concat!("sd ", stringify!($reg), ", {", stringify!($reg), "}(sp)"),)*

                // Every general register is in the frame, so `t0` is free scratch now.
                // Before this line there is none: a store of the interrupted `sp` needs a
                // register to compute it in, and clobbering one first would lose it.
                "addi t0, sp, {frame_size}",
                "sd   t0, {stack}(sp)",
                "csrr t0, sepc",
                "sd   t0, {pc}(sp)",
                "csrr t0, sstatus",
                "sd   t0, {status}(sp)",

                // The frame is the argument, and it is a `&mut`: a handler may edit what
                // the restore below puts back.
                "mv   a0, sp",
                "call {dispatch}",

                "ld   t0, {status}(sp)",
                "csrw sstatus, t0",
                "ld   t0, {pc}(sp)",
                "csrw sepc, t0",
                $(concat!("ld ", stringify!($reg), ", {", stringify!($reg), "}(sp)"),)*

                // Arithmetic, not a load: this frame sits on the interrupted stack, so
                // undoing the subtraction *is* restoring `sp`. The saved copy is there for
                // the dump a fault prints, and for the user-mode return that will load it.
                "addi sp, sp, {frame_size}",
                "sret",

                frame_size = const size_of::<TrapFrame>(),
                stack = const offset_of!(TrapFrame, sp),
                pc = const offset_of!(TrapFrame, sepc),
                status = const offset_of!(TrapFrame, sstatus),
                dispatch = sym super::dispatch,
                $($reg = const offset_of!(TrapFrame, $reg),)*
            )
        }
    };
}
saved_registers!(define_entry);

/// The address to put in `stvec`.
pub(super) fn vector() -> VirtualAddr { VirtualAddr::new(trap_entry as *const () as usize) }
