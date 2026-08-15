//! The boot hart's prologue: the last assembly before ordinary Rust.

use crate::memory::layout;

boot_fn!(
    /// Give Rust a stack, and enter it.
    ///
    /// Reached from `super::entry::enter_high` at a high virtual address, with `a0` the
    /// hart id, `a1` the device tree and `a2` the measured VA offset — already the three
    /// arguments [`crate::start::boot`] takes, in order.
    ///
    /// Nothing else: everything the boot hart still owes before it can trust a static —
    /// zeroing `.bss` above all — is Rust in [`crate::start::boot`], since once `sp` exists
    /// there is no reason for assembly.
    pub(super) fn prologue in entry {
        // The only stack until the frame allocator exists. `.boot_stack` is NOLOAD and
        // outside `.bss`, so the clear that follows will not walk over it.
        "la sp, {boot_stack_end}",
        // `boot` spills `ra` on entry as any non-leaf function does, and this is the
        // outermost frame, so zero is what stops the unwind gdb walks out of it.
        "mv ra, zero",
        "tail {boot}",
    }
        boot_stack_end = sym layout::_boot_stack_end,
        boot = sym crate::start::boot,
);
