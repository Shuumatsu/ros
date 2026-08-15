//! The assembler options every block in this stage runs under.

/// Assemble a boot-stage naked function body: instructions in braces, `asm!` operands
/// after.
///
/// `norvc`, because the kernel is built with the C extension (see `.cargo/config.toml`)
/// and this stage counts instruction bytes: the Image header's `code0` field is exactly
/// four, and a compressed `j` would slide every field after it.
///
/// `norelax`, because these blocks run before `gp` is set. Left relaxable, the linker
/// rewrites `la gp, __global_pointer$` into a `gp`-relative load of `gp` itself, and any
/// other global access with it.
macro_rules! boot_asm {
    ({ $($insn:literal),* $(,)? } $($operands:tt)*) => {
        ::core::arch::naked_asm!(
            ".option push",
            ".option norvc",
            ".option norelax",
            $($insn,)*
            ".option pop",
            $($operands)*
        )
    };
}
