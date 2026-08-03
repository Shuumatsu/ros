use crate::trap::TrapFrame;

/// Supervisor handler for `ecall` from U-mode — the future system-call entry.
///
/// No user programs or system calls exist yet (they arrive with user-process
/// support), so any `ecall` reaching here is unexpected: report it and return an
/// error in `a0` rather than pretend to service it. A real dispatch table will
/// replace this body.
pub fn handler(tf: &mut TrapFrame) {
    // The locked writer, not the emergency one: an `ecall` is executed deliberately,
    // so it can never arrive while this hart is inside `_print` holding the lock —
    // which is the only thing the emergency path exists for.
    println!("[ecall] unexpected syscall {} — none implemented yet", tf.a7);
    tf.a0 = usize::MAX;
    tf.increase_sepc();
}
