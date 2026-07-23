use core::arch::naked_asm;
use core::mem::size_of;
use core::ptr::addr_of_mut;

use crossbeam_utils::CachePadded;
use riscv::register::{mie, mscratch, mstatus, mtvec};

use crate::arch::{NCPU, riscv64::hart_id};
use crate::device_tree::clint_base;
use crate::platform::{CLINT_MTIME_OFFSET, CLINT_MTIMECMP_OFFSET};
use crate::trap::TrapFrame;

pub const INTERVAL: u64 = 10_0000;

/// Address of this hart's `mtimecmp` register (CLINT base from the device tree).
#[inline]
fn mtimecmp_addr(hart: usize) -> usize {
    clint_base() + CLINT_MTIMECMP_OFFSET + 8 * hart
}

/// Address of the CLINT `mtime` register.
#[inline]
fn mtime_addr() -> usize {
    clint_base() + CLINT_MTIME_OFFSET
}

// prepare information in scratch[] for timervec.
// scratch[0]: address of CLINT MTIMECMP register.
// scratch[1]: desired interval (in cycles) between timer interrupts.
//     因为 addi 要求 [-2048, 2047]，所以我们得用一个寄存器来存 interval
// scratch[2..4] : space for timervec to save registers.
type Scratch = (u64, u64, u64, u64, u64);
const_assert_eq!(size_of::<Scratch>(), 5 * size_of::<u64>());
static mut TIMER_SCRATCH: [CachePadded<Scratch>; NCPU] = [CachePadded::new((0, 0, 0, 0, 0)); NCPU];

#[unsafe(naked)]
unsafe extern "C" fn timervec() {
    naked_asm!(
        "
            csrrw a0, mscratch, a0

            # save a1, a2 and a3
            sd a1, 16(a0)
            sd a2, 24(a0)
            sd a3, 32(a0)

            ld a1, 0(a0) # mtimecmp addr
            ld a2, 0(a1) # mtimecmp
            ld a3, 8(a0) # interval

            # schedule the next timer interrupt
            add a2, a2, a3 # next mtimecmp
            sd a2, 0(a1)

            # RISC-V要求在机器模式下处理定时器中断，而不是监督者模式;
            # 监督者模式下的 timer interrupt 需要在机器模式下通过设置 sip 软件触发
            # raise a supervisor software interrupt.
            li a1, 2
            csrw sip, a1

            # restore a1, a2 and a3
            ld a1, 16(a0)
            ld a2, 24(a0)
            ld a3, 32(a0)

            csrrw a0, mscratch, a0

            mret"
    );
}

// each CPU has a separate source of timer interrupts.
pub unsafe fn init() {
    let hart = hart_id();

    let scratch: *mut CachePadded<Scratch> = unsafe { addr_of_mut!(TIMER_SCRATCH[hart]) };
    unsafe {
        (&mut *scratch).0 = mtimecmp_addr(hart) as u64;
        (&mut *scratch).1 = INTERVAL;
        mscratch::write(scratch as usize);
    }

    // RISC-V uses 2 memory-mapped registers mtime and mtimecmp to control timer interrupts.
    // ask the CLINT for a timer interrupt.
    let mtimecmp = mtimecmp_addr(hart) as *mut u64;
    let mtime = mtime_addr() as *const u64;
    unsafe { mtimecmp.write_volatile(mtime.read_volatile() + INTERVAL) };

    unsafe { mtvec::write(mtvec::Mtvec::new(timervec as *const () as usize, mtvec::TrapMode::Direct)) };

    unsafe { mstatus::set_mie() };
    unsafe { mie::set_mtimer() };
}

pub fn handler(tf: &mut TrapFrame) { unimplemented!() }
