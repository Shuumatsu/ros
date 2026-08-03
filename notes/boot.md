# Boot & kernel-initialization architecture

How the kernel comes up, and *why* it's built this way. Read this before touching
`boot.S`, `kernel.ld`, `memory/`, or the trap code.

## Boot chain

```
QEMU reset (M-mode)
  -> OpenSBI            (M-mode firmware, QEMU `-bios default`)
       PMP, trap delegation, timer; mret to S-mode
  -> our kernel         S-mode, a0 = hartid, a1 = dtb, entered at PHYS 0x80200000
```

We are a **pure S-mode payload under OpenSBI**, not an M-mode self-boot (`-bios
none`). Rationale: the Linux RISC-V boot protocol enters the kernel in S-mode
with the SBI owning M-mode. Following it means the kernel has *no* M-mode code
(no PMP/`mret`/`mhartid`/M-timer) — simpler and standard. OpenSBI sets `medeleg`/
`mideleg`, so we don't.

## Image format

The kernel is booted as a **flat RISC-V `Image`** (not the ELF): `boot.S` starts
with the 64-byte Linux image header (`code0 = j _boot`, `text_offset = 0x200000`,
`magic2 = "RSC\x05"`). `scripts/run.sh` `objcopy`s the ELF to a flat binary and
runs QEMU; the cargo runner points at it. We boot the Image (not the ELF) because
the ELF's entry is a high VA that isn't mapped at reset — the flat Image is
loaded at its physical `text_offset` and entered at `code0`.

## Higher-half layout

- Linked to **run** at high VAs: base `0xffffffc000200000` (bottom of the Sv39
  high canonical half + the 2 MiB `text_offset`).
- **Loaded** at physical `0x80200000` (RAM base `0x80000000` + 2 MiB; OpenSBI
  owns the first 2 MiB). `kernel.ld` sets each section's LMA with
  `AT(ADDR(.x) - _va_offset)`.
- `boot.S` runs at the physical address (PC-relative only, pre-paging), builds a
  minimal **3-gigapage** early table, enables Sv39, and jumps to the high alias:
  - `root[0]`   VA `[0,1G)`     -> PA `[0,1G)`   identity (MMIO devices)
  - `root[2]`   VA `[2G,3G)`    -> PA `[2G,3G)`  identity (RAM: kernel, heap, dtb)
  - `root[256]` VA `[high,+1G)` -> PA `[2G,3G)`  the kernel's high-half home

## Addressing convention

`VA = PA + offset`, where `offset = 0xffffffbf80000000` (Sv39 high-half base
`0xffffffc000000000` − RAM base `0x80000000`).

**Single source, derived — not duplicated.** The layout is declared once in
`kernel.ld` (`_phys_base`, `_va_offset`, `_memory_start`); `_va_offset` is used
there *only* for the sections' LMA math (`AT(ADDR(.x) - _va_offset)`). The
*runtime* offset is never hardcoded in Rust or asm: `boot.S` measures it as
`(linked VMA of a label) - (its real PMA)` and passes it to `start()`, which
records it in `memory::VA_OFFSET`; `phys_to_virt`/`virt_to_phys` read that. So
the offset is derived from the actual load vs. the linked layout — change the
layout in `kernel.ld` alone and everything follows. (An earlier version hardcoded
the constant in three files; that split-brain has been removed.)

We keep RAM **identity-mapped** as well as high-mapped, so a `PhysicalAddr` is
still a usable pointer. That's why the `paging` crate and `frame.rs` need **no**
changes for higher-half — `pa.as_ptr()` works via the identity map. `memory.rs`
provides `virt_to_phys`/`phys_to_virt` for the one spot that crosses over (giving
the frame allocator physical bounds while the heap lives at high VAs).

## Memory

- Kernel heap: bounded 8 MiB (`KERNEL_HEAP_SIZE`) at high VAs, right after the
  kernel image + stack. Holds kernel bookkeeping (incl. the frame allocator's
  free lists), so it must come up *before* the frame allocator.
- Physical frame allocator: the rest of RAM `[heap_end_pa, ram_end)`, physical.
- `ram_end` is discovered from the device tree (`/memory`), never hardcoded.

## Traps and the timer — PARKED

**There is no trap handler and no timer right now.** The whole subsystem was moved
out of the crate to `crates/kernel/attic/trap/` while boot and memory init are being
finalised; that directory's README says why and what has to happen before it returns.
`stvec` stays on `boot.S`'s `_trap_park` for the life of the kernel, so any trap
parks the faulting hart with `scause`/`sepc`/`stval` intact. `sstatus.SIE` is never
set and no interrupt source is enabled, which makes the `wfi` loops in `kmain` /
`kmain_ap` true halts rather than idles.

What was there, kept here because it is what gets rebuilt:

S-mode timer via the SBI TIME (legacy `set_timer`) extension: `rdtime` for the
current time, `sbi::set_timer(now + INTERVAL)` armed and re-armed each tick, with
`sie.STIE` + `sstatus.SIE` enabled at init.

Note (Phase-0 gotcha): under OpenSBI+Sstc, `stimecmp` starts at 0, so the timer
interrupt is permanently pending until first armed. Never enable `sie.STIE`
without a handler that arms/clears it.

Also fixed at the time: the trap trampoline reserved 32 slots for a 33-field
`TrapFrame`, writing `sepc` one slot past the frame. Harmless for the old U-mode
ecall demo, fatal for kernel-context interrupts. Now 34 slots (16-byte aligned).

## Deliberately deferred / skipped

- **W^X + direct-map + drop-identity** -> the **user-process phase**. Real W^X
  needs no writable alias of `.text`, which means dropping the blanket identity
  map for a proper direct map — and the payoff (freeing the low half for `U=1`
  user pages) only matters once we have per-process address spaces. Do it there,
  once, with the fine-grained per-process table.
- **PIE / KASLR** -> **skipped.** The higher-half trampoline is already
  physically relocatable via the MMU (fixed high VA -> wherever loaded), so PIE
  at a fixed virtual base is a no-op. `.rela.dyn` self-relocation only buys a
  *randomized* virtual base (KASLR), which is security hardening we don't need.

## Known limits

- The 3-gigapage early table maps only 1 GiB of RAM (`[0x80000000, 0xc0000000)`).
  Fine for the QEMU sizes we run (<=512 MiB); add entries for more.
- Single hart (`-smp 1`). Secondary harts are parked in the SBI (HSM); bring them
  up with `sbi_hart_start` when we do SMP.
