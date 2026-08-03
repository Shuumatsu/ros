//! Supervisor interrupt masking.
//!
//! One implementation, because there were two. `console::_print` had a
//! `disable_interrupts`/`restore_interrupts` pair and `memory::kernel_table::switch_to`
//! open-coded the identical read-`sstatus.sie`, conditionally-clear, conditionally-set
//! sequence inline. Two parts of the kernel independently deciding how interrupts get
//! masked is exactly the split-brain the project standards forbid, and the failure
//! mode is not hypothetical: whoever changes one has no reason to look at the other.
//!
//! Taking a closure rather than returning a token is what makes the restore
//! unskippable. The manual pairs could each be defeated by an early return between the
//! two halves, leaving the hart with interrupts masked and no indication why. A guard
//! with a `Drop` impl would read more idiomatically but buy nothing extra here: both
//! profiles set `panic = "abort"`, so nothing unwinds and `Drop` would not run on the
//! panic path anyway. The closure is honest about what it actually guarantees.

use riscv::register::sstatus;

/// Run `f` with supervisor interrupts masked on this hart, then restore the previous
/// state.
///
/// Restores rather than unconditionally enables: callers nest, and the boot path runs
/// with interrupts off long before anything enables them. Unconditionally setting
/// `sie` on the way out would turn "leave it as you found it" into "turn it on", which
/// during memory bring-up would enable interrupts before there is a handler.
pub fn without<R>(f: impl FnOnce() -> R) -> R {
    let was_enabled = sstatus::read().sie();
    if was_enabled {
        // SAFETY: masking interrupts cannot violate memory safety; the only hazard is
        // leaving them masked, which the restore below prevents.
        unsafe { sstatus::clear_sie() };
    }

    let result = f();

    if was_enabled {
        // SAFETY: restoring the bit this function cleared, on the same hart.
        unsafe { sstatus::set_sie() };
    }
    result
}
