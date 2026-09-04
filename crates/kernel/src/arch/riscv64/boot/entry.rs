//! ISA entry points and the low-to-high address transition.
//!
//! Firmware provides the hart ID in `a0`, boot data in `a1`, `satp = 0`, and interrupts
//! disabled. All other registers, including `sp`, `gp`, and `tp`, are undefined. Compiled
//! Rust cannot run until translation and these ABI registers are initialized.

use mmu::VirtualAddr;

use crate::memory::{boot_table, direct_map, layout};

macro_rules! isa_entry {
    ($(#[$doc:meta])* $vis:vis fn $name:ident => $prologue:path) => {
        boot_fn!(
            $(#[$doc])*
            $vis fn $name in entry {
                // `lla` remains PC-relative before translation is enabled.
                "lla  t2, {prologue}",
                "tail {enter_high}",
            }
                prologue = sym $prologue,
                enter_high = sym enter_high,
        );
    };
}

isa_entry!(
    pub(super) fn primary_entry => super::primary::prologue
);

isa_entry!(
    fn secondary_entry => super::secondary::prologue
);

boot_fn!(
    /// Install the boot page table and continue at the kernel's link-time addresses.
    ///
    /// Preserves `a0` and `a1`, initializes `gp` and `tp`, and leaves stack setup to the
    /// selected prologue.
    fn enter_high in entry {
        // Only the physical park-vector address is valid before translation.
        "lla  t0, {park}",
        "csrw stvec, t0",

        // Build `satp` from the physical root and mode template.
        "lla  t0, {table}",
        "srli t0, t0, {root_shift}",
        "li   t1, {satp_template}",
        "or   t0, t0, t1",
        "csrw satp, t0",
        "sfence.vma",

        // Verify that the loaded physical image and linked high alias differ by `VA_OFFSET`
        // before leaving the identity mapping.
        "lla  t0, 2f",
        "ld   t1, 0(t0)",
        "lla  t0, 1f",
        "li   t3, {va_offset}",
        "add  t0, t0, t3",
        "bne  t0, t1, {park}",
        "jr   t1",
        // The preceding `ld` requires this word to be aligned.
        ".balign 8",
        "2:   .dword 1f",
        "1:",

        // Establish ABI state; zero `tp` means no control block has been adopted.
        "la   gp, {global_pointer}",
        "mv   tp, zero",
        // The identity mapping is temporary, so retain the park vector through its high alias.
        "la   t0, {park}",
        "csrw stvec, t0",

        "add  t2, t2, t3",
        "jr   t2",
    }
        park = sym trap_park,
        table = sym boot_table::TABLE,
        root_shift = const boot_table::SATP_ROOT_SHIFT,
        satp_template = const boot_table::SATP_TEMPLATE,
        va_offset = const direct_map::VA_OFFSET,
        global_pointer = sym layout::GLOBAL_POINTER,
);

boot_fn!(
    /// Stackless park vector used before trap dispatch is available.
    fn trap_park in trap {
        // `stvec` uses the low two address bits as its mode.
        ".balign 4",
        ".globl _trap_vector",
        "_trap_vector:",
        "1:",
        "wfi",
        "j 1b",
    }
);

pub(super) fn secondary_entry_address() -> VirtualAddr {
    VirtualAddr::new(secondary_entry as *const () as usize)
}
