use riscv::interrupt::supervisor::Exception;

use crate::trap::TrapFrame;

pub fn handler(_e: Exception, tf: &mut TrapFrame) { tf.increase_sepc(); }
