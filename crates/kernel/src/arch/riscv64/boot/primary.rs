//! Boot-hart prologue.

use crate::memory::layout;

boot_fn!(
    pub(super) fn prologue in entry {
        // `.boot_stack` is NOLOAD and outside `.bss`, so BSS clearing preserves it.
        "la sp, {boot_stack_end}",
        // Terminate stack unwinding at this outermost frame.
        "mv ra, zero",
        "tail {boot}",
    }
        boot_stack_end = sym layout::_boot_stack_end,
        boot = sym crate::start::boot,
);
