//! The boot hart's prologue: the last assembly before ordinary Rust.

use core::arch::naked_asm;

use crate::memory::layout;

/// Give Rust a stack, and enter it.
///
/// Reached from [`super::entry::enter_high`] at a high virtual address, with `a0`
/// the hart id, `a1` the device tree and `a3` the measured VMA-to-LMA skew.
///
/// Nothing else happens here. Everything the boot hart still owes before it can
/// trust a static — zeroing `.bss` above all — is Rust in [`crate::start::boot`],
/// because once `sp` exists there is no longer a reason for it to be assembly.
#[unsafe(naked)]
#[unsafe(link_section = ".text.init.entry")]
pub(super) unsafe extern "custom" fn prologue() {
    naked_asm!(
        ".option push",
        ".option norvc",
        ".option norelax",
        // The only stack there is until the frame allocator exists. `.boot_stack`
        // is NOLOAD and sits outside `.bss`, so it needs no zeroing and the clear
        // that follows will not walk over the frames holding it.
        "la sp, {boot_stack_end}",
        // `a3` is `enter_high`'s output register; `a2` is the third argument.
        "mv a2, a3",
        "tail {boot}",
        ".option pop",
        boot_stack_end = sym layout::_boot_stack_end,
        boot = sym crate::start::boot,
    )
}
