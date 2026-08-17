# Verifying a change

## Commands

```sh
cargo kbuild                                    # build the kernel ELF
cargo krun                                      # objcopy to a flat Image and boot it
cargo test -p paging -p frame-allocator -p heap  # the host-testable crates
```

`cargo krun` needs the explicit target, which the `.cargo/config.toml` alias supplies; that
file explains why a global `[build] target` would break the host tests. QEMU options live in
`scripts/run.sh`, which is the runner.

A boot parks in `wait_forever`, so a non-interactive log needs a timeout:

```sh
cargo kbuild
llvm-objcopy -O binary target/riscv64imac-unknown-none-elf/debug/kernel /tmp/kernel.bin
timeout 20 qemu-system-riscv64 -nographic -no-reboot -machine virt \
    -cpu rv64 -smp 4 -m 128M -bios default -kernel /tmp/kernel.bin
```

## What a good log shows

The boot log is the test. Every line below is a subsystem reporting what it decided, so a
change that alters a number should alter it for a reason you can name.

- **`kernel image layout:`** — the linker's placement. Sections page-aligned, one page of
  guard between the boot stack and free RAM.
- **`direct map: PA .. -> VA ..`** — the window, and where `kernel_va` therefore starts.
- **`reserve:` / `withheld N frames in M reservations`** — what firmware said is not ours.
  A range printed as `outside the pool, skipped` is below the kernel image and was never
  vendable; that is not the same as unreserved.
- **`frame allocator self-test passed`** and **`kernel heap self-test passed`** — aligned,
  zeroed, distinct, re-zeroed on reuse; and allocation across buddy classes with no leak.
- **the region map** — one line per region. `text r-x`, `rodata r--`, everything else
  `rw-`. No `rwx` anywhere: W^X is asserted, but reading it is free.
- **`kernel page table live on this hart`** — every audit passed before this line printed.
- **`hart N online on the kernel page table`**, once per secondary — each ran on a stack
  only the kernel table maps, so this line is also proof the table is right.

## Two things worth watching

**The boot hart is firmware's choice.** Across runs QEMU has picked hart 0 and hart 2. Both
boot identically, which is the check that nothing indexes by hart id — the property `cpu`'s
module doc claims.

**An audit that never fires proves nothing.** The audits are only worth the boot time if they
reject what they claim to. To confirm one, inject the fault it exists to catch, read the
panic, and revert — a temporary bad region, a stack whose guard is mapped, a hand-edited
`satp`. The panic messages name the offending addresses precisely enough to tell a real
failure from the probe.
