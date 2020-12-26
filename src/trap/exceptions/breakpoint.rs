use riscv::register::scause::Exception;

use crate::trap::TrapFrame;

pub fn breakpoint(e: Exception, tf: &mut TrapFrame) { tf.increase_sepc(); }
