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

`memory::direct_map` divides the Sv39 high half, which is all 256 GiB the kernel
has, into two windows:

- `DIRECT_MAP_SPAN` bytes where `VA = PA + offset` — 128 GiB, the reach of
  `phys_to_virt`, and the ceiling on manageable RAM and on any device window;
- everything above it, which `memory::kernel_va` hands out.

The split is a constant rather than a function of installed RAM. Sizing the first
window to RAM would leave a device above RAM aliasing an address the kernel had
also chosen for itself, and both mappings would be individually valid.

`memory::boot_table` constructs an Sv39 root at compile time:

- low virtual addresses are an identity map over the full half, used during the
  first `satp` write;
- high virtual addresses mirror the direct-map window, and no more, so chosen
  kernel addresses are unmapped there exactly as in the final table.

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

`memory` reads no platform state. It is handed a `machine::MachineMemory` — RAM
top, foreign ranges, device windows — and rejects there, once, any machine whose
device windows the direct map cannot reach. `device_tree` builds that value; a
board without an FDT means another builder and no change to `memory`.

The boot hart initializes memory in dependency order:

1. validate linker and stack geometry, and the machine description;
2. initialize the physical frame allocator over RAM above the image, withholding
   the described foreign ranges;
3. carve the kernel heap from owned frames — it grows on demand, up to a ceiling
   that is the smaller of a fixed maximum and a share of the pool;
4. allocate one guarded stack per secondary, at addresses taken from the kernel
   virtual-address allocator;
5. build and audit the final kernel page table;
6. switch the boot hart to that table.

The final table removes the identity map, applies per-section W^X permissions,
maps discovered MMIO, maps allocator-owned RAM through the direct map, and maps
each stack separately so its lower guard page remains unmapped.

The table is not write-once: it stays owned as an address space behind a lock, so
later mappings edit the live tree rather than building a second view of it. Every
mapping above the direct map is audited against the virtual-address allocator that
handed it out.

Editing goes through `AddressSpace::edit`, which fences the local TLB afterwards;
walking goes through `AddressSpace::walk`, which cannot write. A bare mapper is
unreachable, because Sv39 caches the absence of a translation and an unfenced leaf
is one the hardware ignores. Fencing another hart's TLB needs an IPI and does not
exist yet.

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
