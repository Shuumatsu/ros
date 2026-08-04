# Boot and kernel initialization

## Contract

The kernel is an S-mode payload for an SBI firmware. It uses ordered SMP boot:

1. Firmware enters one boot hart with `satp = 0`, `a0 = hartid`, and `a1 = DTB`.
2. The boot hart initializes the kernel and starts secondaries through SBI HSM.
3. HSM enters the physical secondary entry with `satp = 0`, `a0 = hartid`, and
   `a1 = opaque`.

The older spin-wait protocol, where firmware releases every hart at the Image
entry, is intentionally unsupported.

## Image and address space

The build produces a flat RISC-V Linux `Image`. The 64-byte header and `_start`
live in `arch/riscv64/boot/image.rs`; the linker asserts that the header is exact
and starts at `_memory_start`.

The image is linked at `0xffffffc080200000` and loaded at physical
`0x80200000`. `kernel.ld` defines the fixed skew:

```text
VA = PA + 0xffffffc000000000
```

`memory::boot_table` constructs an Sv39 root at compile time. It maps the full
256 GiB representable by either canonical half twice:

- low virtual addresses are an identity map used during the first `satp` write;
- high virtual addresses are the kernel direct map.

The architecture entry measures its linked-to-physical skew before jumping high.
The first ordinary Rust entry checks that measurement against
`memory::direct_map::VA_OFFSET`.

## Rust boundary

`arch/riscv64/boot/entry.rs` contains `extern "custom"` naked functions. They are
not callable Rust functions and may run without a stack. This layer only:

- installs physical and high trap parking vectors;
- activates the compile-time boot table;
- transfers execution to the linked high alias;
- initializes `gp` and the first stack;
- clears BSS before Rust assumes statics are initialized;
- switches secondaries to the final table and their guarded stacks.

Normal Rust begins in `start::boot` or `start::secondary`. CPU identity, `tp`,
device discovery, allocators, page-table policy, and hart startup all live in
Rust.

## Memory initialization

The boot hart initializes memory in dependency order:

1. validate linker and stack geometry;
2. initialize the physical frame allocator from the DTB RAM range;
3. carve the bounded kernel heap from owned frames;
4. allocate one guarded stack per secondary;
5. build and audit the final kernel page table;
6. switch the boot hart to that table.

The final table removes the identity map, applies per-section W^X permissions,
maps discovered MMIO, maps allocator-owned RAM through the direct map, and maps
each stack separately so its lower guard page remains unmapped.

## Secondary handoff

Each secondary has a dedicated handoff containing the final `satp`, stack top,
and prepared `Cpu` pointer. The boot hart fills the record and release-publishes
its readiness before `sbi_hart_start`.

The stackless secondary entry waits for readiness, performs an acquire fence,
loads the coherent handoff, switches to the final table, installs `sp`, and
tail-transfers to Rust. Rust then installs `tp` and validates the SBI hart ID
against the selected `Cpu`.

Hart IDs are opaque machine identifiers, never array indices. Slot 0 belongs to
the firmware-selected boot hart; secondaries use dense logical slots assigned by
the boot hart.

## Traps

The trap subsystem remains parked. Every hart's `stvec` points at a stackless
`wfi` loop before ordinary Rust runs. Interrupts remain disabled, so reaching the
vector means an early-boot defect and preserves `sepc`, `scause`, and `stval` for
debugging.

## Binary invariants

Debug and release ELFs must retain:

- `_start == _memory_start` and an exact 64-byte Image header;
- no dynamic relocations;
- only PC-relative symbol discovery before the first `satp` write;
- no ordinary ABI entry before `gp`, an aligned stack, and cleared BSS;
- a final-table switch before a secondary adopts its guarded stack.
