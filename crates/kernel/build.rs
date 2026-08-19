//! Hand the kernel's linker script to the linker.
//!
//! Per package rather than per target. `.cargo/config.toml` applies its settings to every crate
//! built for `riscv64imac-unknown-none-elf`, and the user programs under `user/` are built for
//! that same target with linker scripts of their own — a script named there would be forced on
//! all of them.

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let script = format!("{manifest}/src/arch/riscv64/kernel.ld");

    println!("cargo::rustc-link-arg=-T{script}");
    println!("cargo::rerun-if-changed={script}");
}
