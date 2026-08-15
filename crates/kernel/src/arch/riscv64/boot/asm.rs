//! What a boot-stage function is: the ABI it uses, the section it lands in, and the
//! assembler options it is built under.

/// Define a boot-stage naked function: instructions in braces, `asm!` operands after.
///
/// `in header` / `in entry` / `in trap` picks the section. `kernel.ld` orders the stage by
/// those three names, and this is the only place each one is spelled.
///
/// Two assembler options apply to every block.
///
/// `norvc` fixes every instruction at four bytes. The Image header's `code0` field is one
/// instruction wide and a compressed `j` would slide every field after it; the kernel is
/// built with the C extension (see `.cargo/config.toml`), so the assembler takes the short
/// form unless told not to.
///
/// `norelax` keeps the linker from rewriting those instructions afterwards. Relaxation
/// shortens them, which the header's byte count cannot absorb, and it turns
/// `la gp, __global_pointer$` into a `gp`-relative load of `gp` itself — the one access
/// that resolves while `gp` still holds whatever firmware left in it. This target emits no
/// `R_RISCV_RELAX`, so the option is what keeps that a property of the build rather than
/// something the stage rests on.
macro_rules! boot_fn {
    (@define $section:literal, $(#[$attr:meta])* $vis:vis fn $name:ident
        { $($insn:literal),* $(,)? } $($operands:tt)*
    ) => {
        $(#[$attr])*
        #[unsafe(naked)]
        #[unsafe(link_section = $section)]
        $vis unsafe extern "custom" fn $name() {
            ::core::arch::naked_asm!(
                ".option push",
                ".option norvc",
                ".option norelax",
                $($insn,)*
                ".option pop",
                $($operands)*
            )
        }
    };
    ($(#[$attr:meta])* $vis:vis fn $name:ident in header $($rest:tt)*) => {
        boot_fn!(@define ".text.init.header", $(#[$attr])* $vis fn $name $($rest)*);
    };
    ($(#[$attr:meta])* $vis:vis fn $name:ident in entry $($rest:tt)*) => {
        boot_fn!(@define ".text.init.entry", $(#[$attr])* $vis fn $name $($rest)*);
    };
    ($(#[$attr:meta])* $vis:vis fn $name:ident in trap $($rest:tt)*) => {
        boot_fn!(@define ".text.init.trap", $(#[$attr])* $vis fn $name $($rest)*);
    };
}
