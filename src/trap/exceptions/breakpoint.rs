use riscv::register::scause::Exception;

use crate::trap::TrapFrame;

pub fn handler(e: Exception, tf: &mut TrapFrame) { tf.increase_sepc(); }
