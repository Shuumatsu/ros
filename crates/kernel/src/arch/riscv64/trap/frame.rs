//! What a trap saves, the assembly that saves it, and the assembly that puts it back.
//!
//! One frame, not one per privilege level. A supervisor trap, a trap from user mode and a
//! process's first entry into user mode all mean "the register file as the interrupted code left
//! it", and two types spelling that would be two answers to one question. So every general
//! register is here, not only the caller-saved ones a Rust handler would otherwise clobber: the
//! extra stores are what let a context switch and a fault dump read the same frame.
//!
//! The register list is stated once, in [`saved_registers`]. The frame's fields, the stores that
//! fill it, the loads that undo them and the dump a fault prints are all expansions of that one
//! list, so a slot cannot disagree with the instruction that writes it.
//!
//! `sscratch` carries this hart's [`Cpu`] for as long as the kernel runs, which is what lets one
//! vector serve both privilege levels: a single `csrrw` hands [`trap_entry`] the control block
//! *and* somewhere to leave the interrupted stack pointer, and `sstatus.SPP` says which of the
//! two stacks the frame belongs on. The hardware answers "which mode did this come from", so the
//! kernel keeps no flag of its own that could disagree.

use core::fmt;
use core::mem::offset_of;

use riscv::register::sstatus;

use mmu::VirtualAddr;

use crate::cpu::Cpu;

/// `sstatus.SPP`, set when the interrupted context was in supervisor mode.
///
/// Stated as a bit because [`trap_entry`] tests it in assembly; [`TrapFrame::from_user`] asks the
/// same question of the saved copy and reads the same constant, so the two cannot part ways.
const SPP: usize = 1 << 8;

/// The general registers a trap saves, in x-register order.
///
/// `sp` is absent, and is the one field [`TrapFrame`] declares by hand: no instruction naming it
/// can store it, because moving `sp` is what created the frame it would be stored into. The entry
/// recovers it from `sscratch` and the restore loads it last.
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
        /// `align(16)` rather than a padding field: a frame is placed by subtracting this type's
        /// size from a stack pointer, and the RISC-V ABI keeps `sp` 16-byte aligned at all times,
        /// so the alignment is what rounds the size up and nothing has to name the slack.
        ///
        /// `Default` is the all-zero register file [`TrapFrame::for_user`] starts a process from.
        #[repr(C, align(16))]
        #[derive(Clone, Copy, Default)]
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

impl TrapFrame {
    /// The frame a process's first entry into user mode returns from: a trap that never happened.
    ///
    /// Every general register is zero, so nothing the kernel was holding reaches the process, and
    /// `sp` is the only one the ABI requires a value in.
    ///
    /// `sstatus` is the running value with `SPP` cleared and `SPIE` set, rather than a word
    /// composed from scratch: the fields the hardware fixes — `UXL` above all — have to survive
    /// the write [`resume`] makes, and those two bits are the whole of what a return to user mode
    /// changes. `FS` is off and stays off: this kernel saves no floating-point state, and the
    /// target it builds for has no floating-point registers to save.
    pub fn for_user(entry: VirtualAddr, stack_top: VirtualAddr) -> Self {
        let mut status = sstatus::read();
        status.set_spp(sstatus::SPP::User);
        status.set_spie(true);
        status.set_fs(sstatus::FS::Off);

        Self {
            sepc: entry.bits(),
            sstatus: status.bits(),
            sp: stack_top.bits(),
            ..Self::default()
        }
    }

    /// Whether the context this frame holds was running in user mode.
    pub fn from_user(&self) -> bool { self.sstatus & SPP == 0 }
}

macro_rules! define_entry {
    ($($reg:ident),*) => {
        /// Where every supervisor trap lands: save the interrupted context, dispatch it in Rust,
        /// and leave through [`resume`].
        ///
        /// Which stack the frame goes on is the whole of the difference between the two privilege
        /// levels. A trap from supervisor mode continues on the interrupted stack; a trap from
        /// user mode moves to the running process's kernel stack, because the interrupted `sp` is
        /// the process's own and the kernel must not push onto it.
        ///
        /// Naked and `extern "custom"`: the hardware enters it with no arguments and it leaves
        /// through `sret`, so there is no Rust ABI to honour on either side.
        #[unsafe(naked)]
        unsafe extern "custom" fn trap_entry() {
            ::core::arch::naked_asm!(
                // `stvec` reads the low two bits of its address as a mode, so the vector must be
                // 4-byte aligned. rustc gives each function its own section, so this raises
                // *this* function's alignment; `install` asserts the result rather than trusting
                // the toolchain to keep doing that.
                ".balign 4",

                // One instruction for both halves of the problem: `sp` becomes this hart's
                // control block, and the interrupted `sp` — the one register no store can name —
                // lands somewhere a `csrr` can fetch it back from.
                "csrrw sp, sscratch, sp",
                // The one register the entry borrows. Every other register still holds exactly
                // what the interrupted code left in it.
                "sd    t0, {spill}(sp)",
                "mv    t0, sp",

                // `sp` is scratch until the branch below picks a stack: the interrupted value is
                // in `sscratch`, and the frame has no address yet.
                "csrr  sp, sstatus",
                "andi  sp, sp, {spp}",
                "bnez  sp, 1f",

                // From user mode: the running process's kernel stack.
                "ld    sp, {kernel_stack_top}(t0)",
                "j     2f",
                // From supervisor mode: the interrupted stack is a kernel stack already.
                "1:",
                "csrr  sp, sscratch",
                "2:",
                "addi  sp, sp, -{frame_size}",

                // `sscratch` takes the control block back in the same instruction that recovers
                // the interrupted `sp`, and carries it from here through the return below and
                // into the next trap.
                "csrrw t0, sscratch, t0",
                "sd    t0, {stack}(sp)",
                // The borrowed register, put back before the stores below, so every one of them
                // writes a value the interrupted code left.
                "csrr  t0, sscratch",
                "ld    t0, {spill}(t0)",
                $(concat!("sd ", stringify!($reg), ", {", stringify!($reg), "}(sp)"),)*

                // A trap from user mode arrives with the process's `tp`; the kernel's is this
                // hart's control block, which `cpu::current` dereferences — the console does it
                // on every line. The frame carries the process's copy back.
                "csrr  tp, sscratch",

                "csrr  t0, sepc",
                "sd    t0, {pc}(sp)",
                "csrr  t0, sstatus",
                "sd    t0, {status}(sp)",

                // The frame is the argument, and it is a `&mut`: a handler may edit what the
                // return puts back. Named twice because `dispatch` returns with the argument
                // registers spent.
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
        /// The one implementation of "resume a saved context", used in both directions across the
        /// privilege boundary: [`trap_entry`] tails into it to finish a trap, and a process's
        /// first entry into user mode is a call to it with a [`TrapFrame::for_user`] no trap
        /// produced.
        ///
        /// `sstatus` goes back before the register file, so `sret` reads `SPP` and `SPIE` as the
        /// frame recorded them. That word carries `SIE` clear, which is what keeps an interrupt
        /// from arriving between the write and the return. `sscratch` needs no attention: it
        /// holds this hart's control block already, and never stops.
        ///
        /// # Safety
        ///
        /// `frame` must describe a context this hart can resume: an `sepc` and an `sp` the live
        /// page table maps at the privilege level `sstatus.SPP` names. Nothing below can check
        /// any of it — the last instruction leaves the kernel.
        #[unsafe(naked)]
        pub unsafe extern "C" fn resume(frame: *const TrapFrame) -> ! {
            ::core::arch::naked_asm!(
                // The frame becomes the stack pointer, because `sp` is the one register no load
                // can name: it is restored last, out of the frame it was addressing.
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

/// The address to put in `stvec`.
pub(super) fn vector() -> VirtualAddr { VirtualAddr::new(trap_entry as *const () as usize) }
