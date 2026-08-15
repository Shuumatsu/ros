# Boot-time memory

How the kernel gets from a firmware entry with `satp = 0` to a live, audited kernel page
table on every hart. Physical and virtual memory only — traps, drivers and the filesystem
are elsewhere.

## What lives here, and what does not

This directory holds **only what no single module owns**. Every fact about one module is a
doc comment on that module, and these files point at it rather than copy it — a second copy
is a second answer, and the one in the doc is the one nobody recompiles.

So: if a paragraph here explains what `direct_map` does, it is in the wrong place and
belongs in `memory/direct_map.rs`. What is left over, and what these files are for:

| file | owns |
|---|---|
| `README.md` | the chain of stages across subsystems, and where each is implemented |
| `contracts.md` | the seams, and every fact stated twice plus the check that pins it |
| `gaps.md` | what is deliberately not done, and what closing it needs |
| `verify.md` | how to build and boot it, and what a good boot log shows |

## The chain

No module owns this list: each stage knows only its successor. Paths are relative to
`crates/kernel/src`.

### Boot hart

| stage | where | leaves behind |
|---|---|---|
| Image entry | `arch/riscv64/boot/image.rs` | the 64-byte RISC-V Image header, branching to the entry |
| ISA entry | `arch/riscv64/boot/entry.rs::primary_entry` | the prologue to continue into, in `t2` |
| high transition | `arch/riscv64/boot/entry.rs::enter_high` | boot table live, PC high, `gp`/`tp`/`stvec` as Rust expects, measured VMA↔LMA skew in `a2` |
| prologue | `arch/riscv64/boot/primary.rs::prologue` | `sp` on the linker's boot stack |
| first Rust | `start.rs::boot` | `.bss` zeroed, skew verified, device tree parsed, then memory, then secondaries |
| memory | `memory/mod.rs::init` | frames, heap, stacks, kernel page table — in that order, for reasons its doc gives |

`enter_high` is shared with secondaries, which is why it takes the prologue in a register
rather than branching on which kind of hart it is.

### Secondary hart

| stage | where | leaves behind |
|---|---|---|
| publish | `cpu/mod.rs::start_secondaries` | a filled `SecondaryHandoff`, then SBI `hart_start` at the *physical* `secondary_entry` |
| ISA entry | `arch/riscv64/boot/entry.rs::secondary_entry` → `enter_high` | as above, through the same boot table |
| prologue | `arch/riscv64/boot/secondary.rs::prologue` | kernel table adopted, `sp` on this hart's guarded stack |
| first Rust | `start.rs::secondary` | `tp` installed, hart id checked against the chosen `Cpu` |

The boot table is never retired. A hart that has not started has no translation of its own
to arrive with, so every later hart still enters through it.

## Where the code lives

Inside `memory`, the order is `memory::init`'s and the per-module ownership table is in
`memory/mod.rs`. Neither is repeated here.

Three crates carry the parts that do not need a kernel, which is what makes them testable
on the host rather than only on the way through a boot:

| crate | knows nothing about |
|---|---|
| `paging` | allocators, and how a physical address becomes a pointer — both injected |
| `frame-allocator` | mappings, zeroing and locking — its bitmap comes from the caller |
| `heap` | where memory comes from, and locking — it asks and returns |

Each crate's `lib.rs` says what it deliberately does not do. The kernel binds them in
`memory/frame`, `memory/heap` and `memory/address_space`.
