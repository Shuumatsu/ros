//! Trap-frame save and restore.
//!
//! Every general register is preserved. `sscratch` holds the hart's [`Cpu`] and temporarily
//! receives the interrupted `sp`; `sstatus.SPP` selects the supervisor or process kernel stack.

use core::fmt;
use core::mem::offset_of;

use riscv::register::sstatus;

use mmu::VirtualAddr;

use crate::cpu::Cpu;

const SPP: usize = 1 << 8;

/// Saved registers in x-register order; `sp` is handled separately because it addresses the frame.
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
        /// Interrupted context, aligned to preserve the RISC-V stack invariant.
        #[repr(C, align(16))]
        #[derive(Clone, Copy, Default)]
        pub struct TrapFrame {
            pub sepc: usize,
            pub sstatus: usize,
            pub sp: usize,
            $(pub $reg: usize,)*
        }

        impl fmt::Display for TrapFrame {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "  sepc {:#018x}  sstatus {:#018x}", self.sepc, self.sstatus)?;
                write!(f, "\n  {:>3} {:#018x}", "sp", self.sp)?;

                let mut written = 1;
                $(
                    f.write_str(if written % 4 == 0 { "\n  " } else { "  " })?;
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

impl TrapFrame {
    /// Builds a zeroed register file for first entry into user mode.
    ///
    /// The current `sstatus` supplies hardware-managed fields such as `UXL`; `SPP` is cleared,
    /// `SPIE` is set, and floating-point state remains disabled.
    pub fn for_user(entry: VirtualAddr, stack_top: VirtualAddr) -> Self {
        let mut status = sstatus::read();
        status.set_spp(sstatus::SPP::User);
        status.set_spie(true);
        status.set_fs(sstatus::FS::Off);

        Self { sepc: entry.bits(), sstatus: status.bits(), sp: stack_top.bits(), ..Self::default() }
    }

    pub fn from_user(&self) -> bool { self.sstatus & SPP == 0 }
}

macro_rules! define_entry {
    ($($reg:ident),*) => {
        /// Saves a supervisor trap on the interrupted kernel stack or the process's kernel stack.
        #[unsafe(naked)]
        unsafe extern "custom" fn trap_entry() {
            ::core::arch::naked_asm!(
                // `stvec` uses the low two address bits as its mode.
                ".balign 4",

                // Exchange the interrupted `sp` for this hart's control block.
                "csrrw sp, sscratch, sp",
                // Spill the register used to retain the control-block pointer.
                "sd    t0, {spill}(sp)",
                "mv    t0, sp",

                "csrr  sp, sstatus",
                "andi  sp, sp, {spp}",
                "bnez  sp, 1f",

                // User traps switch to the process's kernel stack.
                "ld    sp, {kernel_stack_top}(t0)",
                "j     2f",
                // Supervisor traps remain on the interrupted stack.
                "1:",
                "csrr  sp, sscratch",
                "2:",
                "addi  sp, sp, -{frame_size}",

                // Restore the control block to `sscratch` while recovering the interrupted `sp`.
                "csrrw t0, sscratch, t0",
                "sd    t0, {stack}(sp)",
                "csrr  t0, sscratch",
                "ld    t0, {spill}(t0)",
                $(concat!("sd ", stringify!($reg), ", {", stringify!($reg), "}(sp)"),)*

                // Kernel code expects `tp` to identify this hart; the frame retains the old value.
                "csrr  tp, sscratch",

                "csrr  t0, sepc",
                "sd    t0, {pc}(sp)",
                "csrr  t0, sstatus",
                "sd    t0, {status}(sp)",

                "mv    a0, sp",
                "call  {dispatch}",
                "mv    a0, sp",
                "tail  {resume}",

                frame_size = const size_of::<TrapFrame>(),
                spp = const SPP,
                spill = const Cpu::TRAP_SPILL,
                kernel_stack_top = const Cpu::KERNEL_STACK_TOP,
                stack = const offset_of!(TrapFrame, sp),
                pc = const offset_of!(TrapFrame, sepc),
                status = const offset_of!(TrapFrame, sstatus),
                dispatch = sym super::dispatch,
                resume = sym resume,
                $($reg = const offset_of!(TrapFrame, $reg),)*
            )
        }
    };
}
saved_registers!(define_entry);

macro_rules! define_resume {
    ($($reg:ident),*) => {
        /// Put `frame` back into the register file and `sret` into the context it describes.
        ///
        /// Restoring `sstatus` before the register file keeps interrupts masked until `sret`.
        ///
        /// # Safety
        ///
        /// `frame` must describe a resumable context whose `sepc` and `sp` are mapped for the
        /// privilege level selected by `sstatus.SPP`.
        #[unsafe(naked)]
        pub unsafe extern "C" fn resume(frame: *const TrapFrame) -> ! {
            ::core::arch::naked_asm!(
                // Use the frame as `sp` so the interrupted `sp` can be restored last.
                "mv   sp, a0",

                "ld   t0, {status}(sp)",
                "csrw sstatus, t0",
                "ld   t0, {pc}(sp)",
                "csrw sepc, t0",
                $(concat!("ld ", stringify!($reg), ", {", stringify!($reg), "}(sp)"),)*
                "ld   sp, {stack}(sp)",
                "sret",

                stack = const offset_of!(TrapFrame, sp),
                pc = const offset_of!(TrapFrame, sepc),
                status = const offset_of!(TrapFrame, sstatus),
                $($reg = const offset_of!(TrapFrame, $reg),)*
            )
        }
    };
}
saved_registers!(define_resume);

pub(super) fn vector() -> VirtualAddr { VirtualAddr::new(trap_entry as *const () as usize) }
