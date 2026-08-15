//! The boot hart's prologue: the last assembly before ordinary Rust.

use crate::memory::layout;

/// Give Rust a stack, and enter it.
///
/// Reached from `super::entry::enter_high` at a high virtual address, with `a0` the hart
/// id, `a1` the device tree and `a2` the measured VMA-to-LMA skew — already the three
/// arguments [`crate::start::boot`] takes, in order.
///
/// Nothing else: everything the boot hart still owes before it can trust a static —
/// zeroing `.bss` above all — is Rust in [`crate::start::boot`], since once `sp` exists
/// there is no reason for assembly.
#[unsafe(naked)]
#[unsafe(link_section = ".text.init.entry")]
pub(super) unsafe extern "custom" fn prologue() {
    boot_asm!({
        // The only stack until the frame allocator exists. `.boot_stack` is NOLOAD and
        // outside `.bss`, so the clear that follows will not walk over it.
        "la sp, {boot_stack_end}",
        "tail {boot}",
    }
        boot_stack_end = sym layout::_boot_stack_end,
        boot = sym crate::start::boot,
    )
}
