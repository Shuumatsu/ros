use std::fs::{self, read_dir, File};
use std::io::{Result, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

static TARGET_DIR: &str = env!("TARGET_DIR");

fn main() {
    println!("cargo:rerun-if-changed=../user_apps/src/");
    println!("cargo:rerun-if-changed=../user_lib/src/");
    println!("cargo:rerun-if-changed={}", TARGET_DIR);
    insert_app_data().unwrap();
}

fn insert_app_data() -> Result<()> {
    let mut f = File::create("src/link_app.S").unwrap();
    let mut apps: Vec<_> = read_dir("../user_apps/bin")
        .unwrap()
        .into_iter()
        .map(|dir_entry| {
            let mut name_with_ext = dir_entry.unwrap().file_name().into_string().unwrap();
            name_with_ext.drain(name_with_ext.find('.').unwrap()..name_with_ext.len());
            name_with_ext
        })
        .collect();
    apps.sort();

    writeln!(
        f,
        r#"
    .align 3
    .section .data
    .global _num_app
_num_app:
    .quad {}"#,
        apps.len()
    )?;

    for i in 0..apps.len() {
        writeln!(f, r#"    .quad app_{}_start"#, i)?;
    }
    writeln!(f, r#"    .quad app_{}_end"#, apps.len() - 1)?;

    for (idx, app) in apps.iter().enumerate() {
        // rust-objcopy --binary-architecture=riscv64

        let elf = format!("{}/{}", TARGET_DIR, app);
        let bin = format!("{}/{}.bin", TARGET_DIR, app);

        Command::new("rust-objcopy")
            .arg("--binary-architecture=riscv64")
            .arg(&elf[..])
            .arg("--strip-all -O binary")
            .arg(&bin[..])
            .output()?;

        println!("app_{}: {}", idx, app);
        writeln!(
            f,
            r#"
    .section .data
    .global app_{0}_start
    .global app_{0}_end
    .align 3 
app_{0}_start:
    .incbin "{1}"
app_{0}_end:"#,
            idx, bin
        )?;
    }
    Ok(())
}
