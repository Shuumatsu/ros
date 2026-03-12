use crate::trap::TrapFrame;

const SYS_PRINT: usize = 1;
const SYS_EXIT: usize = 2;

pub fn handler(tf: &mut TrapFrame) {
    let syscall_num = tf.a7;

    match syscall_num {
        SYS_PRINT => {
            kprintln!("[ecall] SYS_PRINT: {}", tf.a0);
            tf.a0 = 0;
            tf.increase_sepc();
        }
        SYS_EXIT => {
            kprintln!("[ecall] SYS_EXIT");
            // Restore kernel context saved by run_user_program.
            // This jumps back to the caller of run_user_program — does not return.
            unsafe { crate::trap::return_to_kernel() };
        }
        _ => {
            kprintln!("[ecall] unknown syscall: {}", syscall_num);
            tf.a0 = usize::MAX;
            tf.increase_sepc();
        }
    }
}
