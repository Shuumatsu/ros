# ros

A RISC-V kernel written from scratch in Rust. It boots as a flat Linux-protocol `Image`
under OpenSBI, installs its own page table to reach the Sv39 high half, and brings every
hart up on one kernel page table.

Target: `riscv64imac-unknown-none-elf`. Machine: QEMU `virt`.

## Prerequisites

| what | why | install |
|---|---|---|
| `qemu-system-riscv64` | the machine, and the OpenSBI firmware it ships | `brew install qemu` |
| `llvm-objcopy` | ELF → flat `Image`; the runner calls it by name | `brew install llvm` |
| Rust nightly + the riscv target | `rust-toolchain` pins both | rustup installs them on the first build |

Homebrew's `llvm` is keg-only, so put it on `PATH` — `export PATH="$(brew --prefix llvm)/bin:$PATH"` — or `llvm-objcopy` will not be found.

## Build and run

```sh
cargo kbuild   # build the kernel ELF
cargo krun     # objcopy it to a flat Image and boot it under QEMU
```

Both are aliases in `.cargo/config.toml`, and both spell `--target` out because that is what
selects the runner. Plain `cargo run -p kernel` builds the riscv ELF and then tries to exec
it on the host.

Run from the repository root. `scripts/run.sh` is the runner — it holds every QEMU option and
names `hdd.dsk` relatively.

Once booted, the QEMU monitor is multiplexed onto the serial console (`-serial mon:stdio`):

| keys | effect |
|---|---|
| `Ctrl-A` `x` | quit QEMU |
| `Ctrl-A` `c` | switch between the console and the monitor |
| `Ctrl-A` `h` | list the rest |

Every hart ends in an idle loop on the timer rather than shutting down, so the kernel never
exits on its own and keeps taking interrupts while it waits.
To capture a log instead of watching one:

```sh
timeout 20 cargo krun > /tmp/boot.log 2>&1
```

## What the runner asks QEMU for

The options that change behaviour, rather than every option in the file:

| option | why |
|---|---|
| `-machine virt -cpu rv64 -m 128M` | the board the device-tree walk and the direct map are sized against |
| `-smp 4` | secondaries are brought up over SBI HSM; one hart hides every ordering bug |
| `-bios default` | QEMU's OpenSBI. The kernel is its S-mode payload, entered at `0x80200000` |
| `-kernel <Image>` | the flat Image, loaded at the RAM base plus `_text_offset` and entered at `code0` |
| `-s` | a gdb server on `:1234`, without waiting for a connection |
| `-d guest_errors,unimp` | QEMU's own complaints, which is where the `pmpaddr` and `HSXLEN` lines come from — not the kernel |
| `-no-reboot` | a triple fault stops instead of looping |

The ELF is not what boots. It is linked at high virtual addresses, so its entry point is an
unmapped VA; the flat Image is loaded physically and installs the boot page table itself,
which also exercises the Image header parsing.

## The disk image

`hdd.dsk` is a 32 MiB raw image, tracked in the repository and attached as a virtio-blk
device. QEMU will not start without it. It is zero-filled and no block driver claims it yet,
so nothing reads it during boot.

`mkfs` builds and inspects an `rfs` image on the host:

```sh
cargo run -p mkfs -- create fs.img 32 ./some-dir   # format, optionally packing a directory
cargo run -p mkfs -- ls   fs.img                   # list the tree
cargo run -p mkfs -- cat  fs.img /path             # dump a file
```

`create` truncates its target, so name a scratch file unless you mean to replace `hdd.dsk`.

## Debugging

`-s` leaves a gdb server on `:1234` for the whole run.

```sh
brew install riscv64-elf-gdb
riscv64-elf-gdb target/riscv64imac-unknown-none-elf/debug/kernel -ex 'target remote :1234'
```

Symbols are at high VAs. Until `enter_high` installs the boot table, the PC is physical, so
early-boot breakpoints need the skew subtracted: `_va_offset` is `0xffffffc000000000`, which
puts the load base at `0x80200000` and the same code at `0xffffffc080200000` afterwards.

## Tests

```sh
cargo test-host
```

`mmu`, `frame-allocator`, `buddy-heap`, `rfs` and `blockdev` carry the parts that do not
need a kernel, and they test on the host. The alias excludes `kernel` and the user programs
because their `forced-target` has no `test` crate; naming them in `cargo test` directly fails
with `can't find crate for 'test'`.

The boot log is the other test: every line is a subsystem reporting what it decided, and a
good one ends with every hart online, `hello, world` from user mode, and the timer still
ticking afterwards.

## Where things are

| path | holds |
|---|---|
| `crates/kernel` | the kernel. `arch/riscv64` holds the boot path and the linker script |
| `crates/mmu` | Sv39 page tables, with allocation and PA→pointer injected |
| `crates/frame-allocator`, `crates/buddy-heap` | physical frames and the kernel heap |
| `crates/rfs`, `crates/blockdev`, `crates/mkfs` | the filesystem, its block layer, and the host tool |
| `crates/abi` | the kernel-user contract: system-call numbers, and the calling side |
| `user/hello` | the first user program, loaded as an ELF and run once at boot |
