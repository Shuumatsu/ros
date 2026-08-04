//! The ISA entry points, and the low-to-high transition they share.
//!
//! # What the firmware hands over
//!
//! `a0` is this hart's id, and `a1` is either the device tree (the boot hart, per
//! the RISC-V boot protocol) or the `opaque` from `sbi_hart_start` (a secondary,
//! per SBI HSM). `satp` is zero and `sstatus.SIE` is clear. **Every other register
//! is undefined** — `sp`, `gp` and `tp` included.
//!
//! # Why none of this is Rust
//!
//! There is no stack to call one with, and no compiled function can be trusted
//! before [`enter_high`] finishes: `medany` makes most references PC-relative, but
//! a jump table, a vtable or a `&'static str` in `.rodata` is an *absolute*
//! link-time address, and every one of those is unmapped until translation is on.
//! So this file does exactly the work that has to happen first, and hands off the
//! moment there is a stack.

use core::arch::naked_asm;

use paging::VirtualAddr;

use crate::memory::boot_table;

unsafe extern "C" {
    #[link_name = "__global_pointer$"]
    static GLOBAL_POINTER: u8;
}

/// Where the boot hart lands, from `_start`'s branch in the Image header.
#[unsafe(naked)]
#[unsafe(link_section = ".text.init.entry")]
pub(super) unsafe extern "custom" fn primary_entry() {
    naked_asm!(
        ".option push",
        ".option norvc",
        ".option norelax",
        "lla t2, {prologue}",
        "tail {enter_high}",
        ".option pop",
        prologue = sym super::primary::prologue,
        enter_high = sym enter_high,
    )
}

/// Where a hart started by [`crate::cpu::start_secondaries`] lands.
///
/// SBI is given the *physical* address of this, because a starting hart has no
/// translation yet — see [`secondary_entry_address`].
#[unsafe(naked)]
#[unsafe(link_section = ".text.init.entry")]
unsafe extern "custom" fn secondary_entry() {
    naked_asm!(
        ".option push",
        ".option norvc",
        ".option norelax",
        "lla t2, {prologue}",
        "tail {enter_high}",
        ".option pop",
        prologue = sym super::secondary::prologue,
        enter_high = sym enter_high,
    )
}

/// Install the boot page table and continue at the kernel's link-time addresses.
///
/// Knows nothing about which kind of hart it is running on. `t2` carries the
/// physical address of the prologue to continue into, and the entry point above is
/// the one that chose it — so the two paths are named where they differ, rather
/// than multiplexed on a flag here.
///
/// Leaves `a0` and `a1` untouched, `a3` holding the measured VMA-to-LMA skew, and
/// `gp`, `tp` and `stvec` set to what Rust expects. Does *not* set `sp`: which
/// stack this hart gets, and which page table maps it, is the prologue's business.
#[unsafe(naked)]
#[unsafe(link_section = ".text.init.entry")]
unsafe extern "custom" fn enter_high() {
    naked_asm!(
        ".option push",
        ".option norvc",
        ".option norelax",

        // A fault before this line goes back to firmware with nothing to say.
        // Physical, because that is the only address space that exists yet.
        "lla  t0, {park}",
        "csrw stvec, t0",

        // satp = the boot table. Its physical address is a link-time fact, so the
        // mode bits arrive as a const template and the root is folded in here.
        // `boot_table` owns both halves of that recipe and pins them to `Satp` at
        // compile time.
        "lla  t0, {table}",
        "srli t0, t0, {root_shift}",
        "li   t1, {satp_template}",
        "or   t0, t0, t1",
        "csrw satp, t0",
        "sfence.vma",

        // Translation is live, and the next fetch still resolves through the
        // identity half — which is the entire reason that half exists. Label `1:`
        // is then reached two ways: read out of the image, where the linker wrote
        // its absolute high address, and PC-relatively, which yields the physical
        // one. Jump to the first. Their difference is the skew, checked against
        // VA_OFFSET by `direct_map::verify` once there is a stack to check it on.
        "lla  t0, 2f",
        "ld   t1, 0(t0)",
        "lla  t0, 1f",
        "sub  a3, t1, t0",
        "jr   t1",
        ".balign 8",
        "2:   .dword 1f",
        "1:",

        // High. Restore the registers the Rust ABI assumes: `gp` for relaxed global
        // access, and `tp` zeroed — `cpu::current` reads a non-zero `tp` as a live
        // per-hart control block, and the firmware left garbage there.
        "la   gp, {global_pointer}",
        "mv   tp, zero",
        // Re-point the park vector at its high alias; the identity half goes away
        // with the boot table.
        "la   t0, {park}",
        "csrw stvec, t0",

        // Into this hart's prologue, at its own high alias.
        "add  t2, t2, a3",
        "jr   t2",

        ".option pop",
        park = sym trap_park,
        table = sym boot_table::TABLE,
        root_shift = const boot_table::SATP_ROOT_SHIFT,
        satp_template = const boot_table::SATP_TEMPLATE,
        global_pointer = sym GLOBAL_POINTER,
    )
}

/// Where a fault goes before there is a trap subsystem to take it.
///
/// Parks rather than returns: `sepc`, `scause` and `stval` stay put for a debugger,
/// and a wedged hart is visible as one instead of silently resetting through
/// firmware. Touches no stack, because for most of [`enter_high`] there is none.
#[unsafe(naked)]
#[unsafe(link_section = ".text.init.trap")]
unsafe extern "custom" fn trap_park() {
    naked_asm!(".option push", ".option norvc", "1:", "wfi", "j 1b", ".option pop")
}

/// [`secondary_entry`] as the virtual address it is linked at.
///
/// SBI needs the physical one; [`crate::cpu::start_secondaries`] converts, because
/// crossing between the two address spaces is `memory`'s to spell out and this
/// module has no business doing it silently.
pub(crate) fn secondary_entry_address() -> VirtualAddr {
    VirtualAddr::new(secondary_entry as *const () as usize)
}
