//! `mkfs` — build, inspect, and read back an rfs image on the host.
//!
//! A standalone host tool that depends on the `rfs` library (which stays
//! `#![no_std]`); this crate is where all the `std` file I/O lives.
//!
//! Subcommands:
//! ```text
//!   mkfs create <image> <size-MiB> [source-dir]  format; optionally pack a dir
//!   mkfs ls     <image> [path]                   list the tree under path (default /)
//!   mkfs cat    <image> <path>                   write a file's bytes to stdout
//! ```
//!
//! Run with `cargo run -p mkfs -- <args>`.

use std::io::Write;
use std::path::Path;
use std::process::exit;
use std::sync::Arc;

use blockdev::{BLOCK_SIZE, RamDisk};
use rfs::{BlockCacheManager, Fs, InodeType, NAME_MAX};

fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("mkfs: {msg}");
    exit(1);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("create") => create(&args[2..]),
        Some("ls") => ls(&args[2..]),
        Some("cat") => cat(&args[2..]),
        _ => {
            eprintln!("usage:");
            eprintln!("  {0} create <image> <size-MiB> [source-dir]", args[0]);
            eprintln!("  {0} ls     <image> [path]", args[0]);
            eprintln!("  {0} cat    <image> <path>", args[0]);
            exit(2);
        }
    }
}

fn create(args: &[String]) {
    if args.len() < 2 || args.len() > 3 {
        die("create <image> <size-MiB> [source-dir]");
    }
    let image = &args[0];
    let mib: usize =
        args[1].parse().unwrap_or_else(|_| die("size must be an integer number of MiB"));
    let total_blocks = mib * 1024 * 1024 / BLOCK_SIZE;
    let ninodes = (total_blocks / 16).max(128);

    let ram = Arc::new(RamDisk::new(total_blocks));
    let fs = Fs::format(Arc::new(BlockCacheManager::new(ram.clone())), total_blocks, ninodes);

    let (mut dirs, mut files) = (0usize, 0usize);
    if let Some(src) = args.get(2) {
        pack(&fs, Path::new(src), "", &mut dirs, &mut files);
    }
    fs.sync();
    std::fs::write(image, ram.snapshot()).unwrap_or_else(|e| die(format!("writing {image}: {e}")));

    let sb = fs.superblock();
    println!("mkfs: wrote {image} ({mib} MiB, {total_blocks} blocks)");
    println!(
        "      inodes {ninodes}, data {} blocks; packed {dirs} dirs, {files} files",
        sb.data_len
    );
}

/// Recursively copy the host directory `host_dir` into the image under `prefix`.
fn pack(fs: &Fs, host_dir: &Path, prefix: &str, dirs: &mut usize, files: &mut usize) {
    let entries = std::fs::read_dir(host_dir)
        .unwrap_or_else(|e| die(format!("read_dir {}: {e}", host_dir.display())));
    for entry in entries {
        let entry = entry.unwrap_or_else(|e| die(format!("reading directory entry: {e}")));
        let name_os = entry.file_name();
        let Some(name) = name_os.to_str() else {
            eprintln!("mkfs: skip non-UTF8 name under {}", host_dir.display());
            continue;
        };
        if name.len() > NAME_MAX {
            eprintln!("mkfs: skip '{name}' (name exceeds {NAME_MAX} bytes)");
            continue;
        }
        let kind = entry.file_type().unwrap_or_else(|e| die(format!("file_type {name}: {e}")));
        let path = format!("{prefix}/{name}");
        if kind.is_dir() {
            fs.create_path(&path, InodeType::Dir).unwrap_or_else(|| die(format!("mkdir {path}")));
            *dirs += 1;
            pack(fs, &entry.path(), &path, dirs, files);
        } else if kind.is_file() {
            let data =
                std::fs::read(entry.path()).unwrap_or_else(|e| die(format!("read {name}: {e}")));
            let mut file = fs.create_file(&path).unwrap_or_else(|| die(format!("create {path}")));
            file.write(&data);
            *files += 1;
        } else {
            eprintln!("mkfs: skip '{name}' (not a regular file or directory)");
        }
    }
}

/// Load an existing image file into a mounted filesystem.
fn mount(image: &str) -> Fs {
    let bytes = std::fs::read(image).unwrap_or_else(|e| die(format!("reading {image}: {e}")));
    let ram = Arc::new(RamDisk::from_image(bytes));
    Fs::mount(Arc::new(BlockCacheManager::new(ram)))
}

fn ls(args: &[String]) {
    if args.is_empty() || args.len() > 2 {
        die("ls <image> [path]");
    }
    let fs = mount(&args[0]);
    let path = args.get(1).map(String::as_str).unwrap_or("/");
    let inode = fs.resolve(path).unwrap_or_else(|| die(format!("no such path: {path}")));
    if fs.inode_type(inode) == Some(InodeType::Dir) {
        list_tree(&fs, inode, path.trim_end_matches('/'));
    } else {
        println!("{path}");
    }
}

fn list_tree(fs: &Fs, dir: u32, prefix: &str) {
    for entry in fs.dir_list(dir) {
        let full = format!("{prefix}/{}", entry.name());
        let is_dir = fs.inode_type(entry.inode) == Some(InodeType::Dir);
        println!("{full}{}", if is_dir { "/" } else { "" });
        if is_dir {
            list_tree(fs, entry.inode, &full);
        }
    }
}

fn cat(args: &[String]) {
    if args.len() != 2 {
        die("cat <image> <path>");
    }
    let fs = mount(&args[0]);
    let mut file = fs.open(&args[1]).unwrap_or_else(|| die(format!("no such file: {}", args[1])));
    let mut out = Vec::new();
    file.read_to_end(&mut out);
    std::io::stdout().write_all(&out).unwrap_or_else(|e| die(format!("stdout: {e}")));
}
