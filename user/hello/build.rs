//! Hand this program's linker script to the linker. See the kernel's `build.rs`: the script is
//! named per package because the target is shared.

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let script = format!("{manifest}/hello.ld");

    println!("cargo::rustc-link-arg=-T{script}");
    println!("cargo::rerun-if-changed={script}");
}
