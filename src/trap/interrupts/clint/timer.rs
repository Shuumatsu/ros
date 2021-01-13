use core::mem::size_of;
use crossbeam_utils::CachePadded;
use riscv::register::{mie, mscratch, mstatus, mtvec};

use crate::arch::{riscv64::hart_id, NCPU};
use crate::memory::layout::{clint_mtimecmp, CLINT_MTIME};
use crate::trap::TrapFrame;

pub const INTERVAL: u64 = 10_0000;

// prepare information in scratch[] for timervec.
// scratch[0]: address of CLINT MTIMECMP register.
// scratch[1]: desired interval (in cycles) between timer interrupts.
//     因为 addi 要求 [-2048, 2047]，所以我们得用一个寄存器来存 interval
// scratch[2..4] : space for timervec to save registers.
type Scratch = (u64, u64, u64, u64, u64);
const_assert_eq!(size_of::<Scratch>(), 5 * size_of::<u64>());
static mut TIMER_SCRATCH: [CachePadded<Scratch>; NCPU] = [CachePadded::new((0, 0, 0, 0, 0)); NCPU];

#[naked]
unsafe extern "C" fn timervec() {
    asm!(
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

            mret",
        options(noreturn)
    );
}

// each CPU has a separate source of timer interrupts.
pub unsafe fn init() {
    let hart = hart_id();

    let scratch: *mut CachePadded<_> = (&mut TIMER_SCRATCH.as_mut_ptr()).add(hart);
    (*scratch).0 = clint_mtimecmp(hart) as u64;
    (*scratch).1 = INTERVAL;
    mscratch::write(scratch as usize);

    // RISC-V uses 2 memory-mapped registers mtime and mtimecmp to control timer interrupts.
    // ask the CLINT for a timer interrupt.
    let mtimecmp = clint_mtimecmp(hart) as *mut u64;
    let mtime = CLINT_MTIME as *const u64;
    mtimecmp.write_volatile(mtime.read_volatile() + INTERVAL);

    mtvec::write(timervec as usize, mtvec::TrapMode::Direct);

    mstatus::set_mie();
    mie::set_mtimer();
}

pub fn handler(tf: &mut TrapFrame) { unimplemented!() }
