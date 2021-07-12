// Example custom build script.
fn main() {
    println!("cargo:rerun-if-changed=src/*");
    println!("cargo:rerun-if-changed=bin/*");
}
