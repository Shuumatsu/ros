use core::mem::size_of;
use riscv::register::sstatus;
use std::sync::atomic::compiler_fence;

use crate::memory::layout::{TEXT_END, TEXT_START};

#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
pub struct GeneralRegs {
    pub zero: usize,
    pub ra: usize,
    pub sp: usize,
    pub gp: usize,
    pub tp: usize,
    pub t0: usize,
    pub t1: usize,
    pub t2: usize,
    pub s0: usize,
    pub s1: usize,
    pub a0: usize,
    pub a1: usize,
    pub a2: usize,
    pub a3: usize,
    pub a4: usize,
    pub a5: usize,
    pub a6: usize,
    pub a7: usize,
    pub s2: usize,
    pub s3: usize,
    pub s4: usize,
    pub s5: usize,
    pub s6: usize,
    pub s7: usize,
    pub s8: usize,
    pub s9: usize,
    pub s10: usize,
    pub s11: usize,
    pub t3: usize,
    pub t4: usize,
    pub t5: usize,
    pub t6: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct CPU {
    pub hart_id: usize,
}

#[inline(always)]
pub fn examine_cpu() -> CPU {
    let mut ptr: usize = 0;

    unsafe {
        llvm_asm!("mv $0, tp" : "=r"(ptr) ::: "volatile");
        *(ptr as *const _)
    }
}

pub fn without_interrupts<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    let sie = sstatus::read().sie();

    unsafe {
        sstatus::clear_sie();
    }

    let r = f();

    if sie {
        unsafe {
            sstatus::set_sie();
        }
    }

    r
}

/// Returns the current frame pointer or stack base pointer
#[inline(always)]
pub fn fp() -> usize {
    let ptr: usize;

    unsafe {
        llvm_asm!("mv $0, s0" : "=r"(ptr));
    }

    ptr
}

/// Returns the current return address
#[inline(always)]
pub fn ra() -> usize {
    let ptr: usize;

    unsafe {
        llvm_asm!("mv $0, ra" : "=r"(ptr));
    }

    ptr
}

// // Print the backtrace starting from the caller
// pub fn backtrace() {
//     unsafe {
//         let mut current_ra = ra();
//         let mut current_fp = fp();
//         let mut stack_num = 0;

//         println!("=== BEGIN rCore stack trace ===");

//         while current_ra >= *TEXT_START && current_ra <= *TEXT_END && current_fp as usize != 0 {
//             // print current backtrace
//             println!(
//                 "#{:02} PC: {:#018X} FP: {:#018X}",
//                 stack_num,
//                 current_ra - size_of::<usize>(),
//                 current_fp
//             );

//             stack_num = stack_num + 1;

//             current_fp = *(current_fp as *const usize).offset(-2);
//             current_ra = *(current_fp as *const usize).offset(-1);
//         }

//         println!("=== END rCore stack trace ===");
//     }
// }
