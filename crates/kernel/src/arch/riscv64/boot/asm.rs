//! Boot-stage assembly support.

/// Define a boot-stage naked function: instructions in braces, `asm!` operands after.
///
/// `norvc` preserves the Image header's fixed-width `code0` field. `norelax` also prevents
/// `gp` initialization from becoming a `gp`-relative access before `gp` is valid.
///
/// The section names where `kernel.ld` places the function. `header` must be the image's first
/// byte. `trap` is separate because its sole occupant takes the section's alignment as its entry
/// address, which a naked function cannot request for itself.
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
