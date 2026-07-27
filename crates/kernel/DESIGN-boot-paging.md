# Boot, Paging and Memory — Design Notes & Session Handoff

Everything in **§2 Completed** is landed, builds, and boots. **§4** is the open
work to resume on.

Stage 2 (early page-table construction in Rust) is **done** — see §2.D. It was
built as **Option 1, the linear direct map**; the three options that were on the
table, and why 1 won, are preserved in §2.D for the record.

---

## 1. The conceptual model (settled)

Three distinct allocators, often confused. Getting this straight drove all the
work below.

| | Frame allocator (PMM) | `LockedHeap` (kernel heap) | VMA / address-space mgr |
|---|---|---|---|
| Granularity | one page (4 KiB) | arbitrary bytes | region (VA range) |
| Address kind | **physical** | virtual, inside an already-mapped arena | virtual |
| Manages | all physical RAM | one arena carved out of RAM | which VAs are legal |
| Depends on | nothing (by design) | a mapped arena, i.e. the PMM | page tables |
| Status here | `crates/frame-allocator` ✅ | `buddy_system_allocator::LockedHeap` ✅ | **does not exist yet** |

Key corrections to the intuitions we started from:

- **The frame allocator is not "the thing that backs page tables."** It is the
  single record of which physical frames are free. Page tables are *one*
  customer; others are heap backing, user pages, DMA buffers, kernel stacks.
  Mapping one virtual page may take *two* frames from it: the backing frame, and
  possibly a new intermediate page-table frame.
- **`LockedHeap` does not allocate virtual address space.** It sub-divides a
  byte range that is *already mapped*. It neither creates mappings nor tracks the
  address space. That third job (VMAs, mmap, brk, stack growth) has no owner in
  this kernel yet.
- **Page fault → frame allocator → mapping** describes *demand paging*, which is
  one strategy, not the universal path. Kernel mappings are built eagerly at
  boot and never fault. And a fault is not automatically an allocation: the
  handler first consults the VMA to decide *legal or not*, and only then may
  allocate (fresh anonymous page), copy (COW), read from disk (swap), or kill the
  process (genuine segfault).
- **Correct init order is PMM → heap**, with the heap carved out of the PMM. The
  kernel had this backwards; see §2.B for the root cause.

---

## 2. Completed work

### A. `crates/frame-allocator` — simplification + DRY audit

Landed: `thiserror` 2 derives replacing ~65 lines of hand-written
`Display`/`Error`/`From`/`source`; `heapless::Vec<Root, MAX_ROOTS>` replacing the
`[Root; N]` + `root_count` + `Root::EMPTY` triad; `pretty_assertions` in tests.

> **Deliberately rejected:** `bytemuck` (this crate takes `&mut [usize]` and never
> reinterprets bytes — it is the right tool in `rfs` for on-disk POD, wrong here)
> and `bitvec` (the hand-rolled word-masked `Bitmap` is tighter and more auditable
> for kernel code).

One subtlety worth preserving: `InitError::Metadata` uses `#[error("{0}")]`, **not**
`#[error(transparent)]`. `#[from]` already makes the inner error the `source()`;
`transparent` would forward `source()` to the *inner error's* source (`None`) and
silently break the chain. There is a regression test
(`error_messages_and_source_chain_are_stable`) pinning this.

Then an AGENTS.md audit found and fixed two real violations:

1. **`WORD_BITS` defined twice** (`allocator.rs` and `bitmap.rs`). Now
   `pub(crate)` in `bitmap.rs`, the module that owns the "machine word" concept.
2. **The buddy layout was computed twice** — `metadata_layout()` walked
   `range.roots()` applying `2n−1`, then `FrameAllocator::new()` walked it *again*
   re-deriving the same `bit_offset`, with the two reconciled by `debug_assert_eq!`.
   Textbook split-brain, and the guard was **invisible in release**
   (`[profile.release]` has no debug-assertions), so the two could diverge
   unchecked. Collapsed into one private `decompose(range) -> (Vec<Root>, MetadataLayout)`
   that both call. `new()` went from ~30 lines to ~15.

### B. Kernel PMM-first reordering

**Root cause of the backwards order** (worth remembering): the old
`buddy_system_allocator::FrameAllocator` keeps its free lists in `BTreeSet`s —
i.e. *on the heap*. That forced heap-first. Tail wagging the dog: a wrong PMM
choice dictated the boot order.

Fixed at the root by switching to `crates/frame-allocator`, which keeps metadata
in a caller-supplied bitmap and depends only on `core`.

- `memory::init()` is now **PMM first** over `[free_start, ram_end)` where
  `free_start = virt_to_phys(_heap_start)` (top of the kernel image), **then** the
  heap carved from it via `frame::alloc_contiguous(2048)` → 8 MiB.
- The **bitmap is carved from the front of the managed range** (runtime-sized
  from the device tree), and excluded from the frames handed out — satisfying the
  crate's SAFETY contract. No static cap, no compile-time guess about RAM size.
- Frames are vended as a **move-only `Frames` token**; `free` consumes it, so
  double-free is impossible in safe code. The heap's token is deliberately never
  freed, which pins its frames for the kernel's lifetime.
- `LockedHeap` is still `buddy_system_allocator` — only the *frame* allocator
  changed.

### C. `crates/paging` — purified + superpages (Stage 1 of the boot refactor)

`extern crate alloc` is **gone**; verified by compiling `no_std` for host *and*
`riscv64imac-unknown-none-elf`. The allocator dependency was only ever 3
production lines in `table.rs`.

Two policies are now injected, one concern per file:

| File | Concern |
|---|---|
| `sv39/frames.rs` | `unsafe trait FrameSource` — *where* table frames come from |
| `sv39/access.rs` | `unsafe trait PhysAccess` + `Identity` / `LinearOffset` — *how* to reach a frame |
| `sv39/mapper.rs` | `Mapper` + `MapError` — all walk logic |
| `sv39/table.rs` | `Table` — pure data + allocation-free ops only |

Both traits are `unsafe` on purpose: a wrong impl is UB in the walk, not merely a
wrong mapping.

- **Latent higher-half bug fixed as a side effect.** `child_table()` used to do
  `entry.target().as_mut_ptr()` — treating a physical address *as* a pointer. With
  the heap now at `0xffffffc0_00800000`, `PhysicalAddr::new(heap_ptr).ppn()` would
  have produced a garbage PPN and the walk would have dereferenced it. Unused so
  far only because `proc/` is commented out. Now all descent goes through
  `access.ptr::<Table>()`.
- **Superpages**: `map_at_level(va, pa, level, flags)` for 4 KiB / 2 MiB / 1 GiB.
  Branches are created only for levels *above* the target, so a root-level
  mapping is **allocation-free by construction** — proven by a test that maps a
  gigapage with a `Barren` frame source that always returns `None`.
- **Const path for the boot table**: `const fn Entry::leaf`/`branch` and
  `const fn Table::map_gigapage`. A test builds a boot-shaped table (low 4 GiB
  identity + high-half mirror) as a `static` through const evaluation — if it
  weren't genuinely const-usable it wouldn't compile. It *derives* root index 256
  from `VirtualAddr::vpn(2)` instead of hardcoding it.
- **Typed `MapError`** (`InvalidLevel`, `UnalignedVirtual/Physical`, `NotALeaf`,
  `WriteWithoutRead`, `OutOfFrames`, `SuperpageInPath`). These were previously
  `debug_assert!`s — compiled out of the release kernel, so bad mappings would
  have landed silently.
- Documented non-unwind: `OutOfFrames` mid-walk leaves already-created (empty)
  intermediate tables in place. Not corruption; they're reused. Just not rolled back.

### D. Stage 2 — early page tables in Rust, via a linear direct map

`boot.S` no longer knows the Sv39 PTE format, the satp mode, or where RAM is.
The whole early table is a `const`-evaluated `static`.

**The redefinition that made it possible.** `_va_offset` used to be
`_kernel_va_base - _dram_base` = `0xffffffbf80000000` — skewed by the RAM base,
so it only described the kernel *image*. It is now the pure constant
`0xffffffc000000000`, so `VA = PA + _va_offset` holds for **all** of physical
memory. The kernel therefore relinks from `0xffffffc000200000` to
`0xffffffc080200000`.

That one change is what lets the boot table be const-built with **zero DRAM-base
knowledge**: map a fixed window `PA [0,4 GiB)` twice — identity, and at
`VA_OFFSET + PA` — and RAM lands wherever the platform puts it inside it.

> **The sharpest argument for the linear map**, which the original option
> write-up missed: under the old skewed offset, `phys_to_virt(0x1000_0000)` (the
> UART) produced `0xffffffbf90000000`, which is **not a canonical Sv39 address**
> (it is below the `0xffffffc0…` high-half base) and was not mapped anyway. It
> never faulted only because nothing called it on a device address. Now MMIO maps
> to `0xffffffc010000000` inside the `i=0` high-half gigapage, so `phys_to_virt`
> is valid for *every* physical address. That is the precondition for ever
> dropping the boot identity map. Options 2 and 3 both kept the skew and that
> latent trap — which is why they lost.

Landed:

| File | Change |
|---|---|
| `paging/src/satp.rs` **(new)** | `Satp` + `Mode` — the CSR layout, `MODE=8` included. Sits *above* `sv39/` because the RV64 satp format doesn't vary with mode. `const fn new/sv39/with_root`, asserts a page-aligned root and an in-range ASID. 8 tests. |
| `memory/direct_map.rs` **(new)** | The single source: `VA_OFFSET`, `WINDOW_END`, the `const fn early_table()`, `EARLY_PGTABLE`, `EARLY_SATP_TEMPLATE`, `verify()`. |
| `kernel.ld` | `_va_offset` = pure constant; `_kernel_va_base` deleted (it was only ever an input to the skew). |
| `boot.S` | ~25 lines of PTE/RAM-base/satp arithmetic → 7 lines that only point satp at a table. The `.data`/`early_pgtable` `.space 4096` block is gone. |
| `memory/mod.rs` | `VA_OFFSET: AtomicUsize` + `set_va_offset` + `va_offset()` **deleted**; `phys_to_virt`/`virt_to_phys` are now `const fn`. |
| `memory/frame.rs` | Window clamp reads `direct_map::WINDOW_END` instead of re-deriving `ram_base + 1 GiB`. |
| `start.rs` | `set_va_offset(va_offset)` → `direct_map::verify(va_offset)`. |

Details worth preserving:

- **The table lives in `.rodata`, not `.data`.** Every leaf pre-sets `A`/`D`, so
  the hardware walker never writes to it, which makes it genuinely immutable.
  `.rodata` is PROGBITS in the first `LOAD` segment, so the loader materializes it
  and `boot.S` can reach it PC-relative before paging. (The old asm table *had* to
  be in `.data` to be pre-zeroed; a const table needs no zeroing at all.)
- **The one irreducible line of asm arithmetic** is `srli t2, t0, 12` — satp's
  PPN is the table's page number, and the table's address is a link-time fact no
  `const` can see. Everything else comes from the `EARLY_SATP_TEMPLATE`
  (`MODE=Sv39, ASID=0, PPN=0`), which `boot.S` just `or`s the PPN into.
  `Satp::with_root` is the same operation in Rust, and a test pins the two to the
  same bit pattern.
- **`VA_OFFSET` is still duplicated in `kernel.ld`** — unavoidable, the linker
  cannot read a Rust `const` and a `build.rs` linker-script parser (Option 3) is
  worse glue. What makes it safe: `boot.S` already computes the *real* VA−PA skew
  for its jump-high (`.Lhigh_entry` VMA − `lla` PMA), and `direct_map::verify()`
  asserts it equals `VA_OFFSET`. This checks **reality**, not just the linker's
  intent, so it also catches a loader that put us somewhere unexpected. It is a
  plain `assert_eq!`, so it is live in release too.
- **The guard was negative-tested**, not assumed: injecting `addi a2, a2, 8`
  after `boot.S`'s `sub` (corrupting only the measurement, not the mapping) makes
  the kernel panic at boot with
  `boot.S measured a VA offset of 0xffffffc000000008, but the direct map is built
  for 0xffffffc000000000`. Reverted and the binary re-verified clean by
  disassembly.
- **No Rust runs before paging is on.** The table is *data*, so the pre-paging
  codegen hazards flagged in §3 (linker relaxation to `gp`-relative, absolute
  relocations) are sidestepped rather than mitigated. Confirmed by disassembly:
  the whole pre-`satp` path is `auipc`/`addi`, and `llvm-readelf -r` reports **no
  relocations in the image at all**.
- `page_size_at(ROOT_LEVEL)` supplies the gigapage size; `WINDOW_GIGAPAGES = 4` is
  the only tunable, and `WINDOW_END` is derived from it.

**Also fixed here (rot the previous handoff missed):** `memory/mod.rs`'s
`KERNEL_HEAP_SIZE` comment still described the *pre-reorder* world — "the physical
frame allocator, which **cannot exist until this heap does** (it keeps its free
lists here)… `frame`'s `BTreeSet`s included." That was the dead
`buddy_system_allocator` rationale, and it flatly contradicted the correct
`init()` doc 30 lines below it. Commit `a7c9cd9` fixed the code and left the
comment lying.

### E. Stage 3 — the kernel's own page table, with W^X

The kernel no longer runs on the boot table. `memory/kernel_table.rs` builds a
real one and switches `satp` to it during `memory::init`.

| Region | rights | granularity |
|---|---|---|
| `.text` | **r-x** | 4 KiB |
| `.rodata` | **r--** | 4 KiB |
| `.data` / `.bss` / kernel stack / frame-pool head | rw- | 4 KiB |
| bulk direct map | rw- | **2 MiB** |
| MMIO (low 1 GiB) | rw- | 1 GiB |

Before this, *every* mapping was a 1 GiB `RWX` gigapage: `.text` was writable and
`.rodata` executable. Paging was on and buying nothing.

**Enabling change in `frame-allocator` (commit `9bcdb1f`).** `deallocate_at(start,
order)` — freeing by address, for a caller whose only surviving handle is a PTE.
Built on a private `block_at` that reconstructs the node index as the exact
inverse of `allocate`'s position arithmetic, then hands the rebuilt token to
`deallocate`, so coalescing and the accounting guard stay in one place. What it
can and cannot catch is the interesting part:

- double free → `AlreadyFree` (the ancestor scan finds whatever swallowed the
  block when it coalesced)
- unmanaged frame, order beyond its root → `ForeignBlock`, the latter checked
  *before* any `1 << order` so an absurd order cannot overflow the shift
- start that cannot begin that order → new `UnalignedFrame`
- **order not matching the original allocation → undetectable.** The bitmap
  records the extent of *free* blocks, never allocated ones. This is why the
  function is `unsafe` and why the token-based `deallocate` remains the default.

**Design points worth keeping:**

- **One layout, consumed twice.** `regions()` computes the layout once; it is
  iterated to *install* and again to *verify*. A separate list of expectations
  would be a second encoding free to drift, and the drift wouldn't surface until
  something faulted.
- **Verification precedes the switch, and covers every page** — not a sample.
  `Mapper::entry_of` (added for this: `translate` discards the flags) checks
  level, flags and target frame for all ~570 pages, plus the stack/heap guard is
  still a hole, plus the **running PC and SP** read out of the live machine with
  `auipc`/`mv`. Mis-mapping `.text` faults on the instruction *after* `csrw satp`
  with the old table gone — unrecoverable and nearly undiagnosable.
- **The kernel image's superpage slot is mapped at 4 KiB, everything above it at
  2 MiB.** The image lives *inside* the direct map, so a gigapage over it would
  make per-section rights impossible (`Mapper` would return `SuperpageInPath`).
- **OpenSBI's RAM is deliberately not mapped.** Its PMP dump says
  `0x80000000-0x8004ffff … S/U: ()` — no S-mode access at all. The direct map
  starts at the kernel image, not at `ram_base`.
- `A` is pre-set everywhere so the walker never writes back into a table; `D`
  only where writable, since it means "has been written".
- `GLOBAL` deliberately omitted — a TLB optimisation whose correctness depends on
  address spaces that don't exist yet.
- Interrupts are masked across `csrw satp; sfence.vma` so no trap observes a
  half-switched translation. (Timer interrupts *are* live by then.)

**W^X was verified empirically, not assumed.** A temporary probe writing to
`_text_start` after the switch produced `scause 15` / `StorePageFault`, and the
"probe failed" line never printed. Under the boot table that write would have
silently succeeded. Probe reverted and the binary re-verified.

Two guard pages fall out of the layout for free, because the region list simply
doesn't cover them: the `.rodata`/`.data` alignment slack, and the page the
linker reserves between the kernel stack and the heap
(`_heap_start = _kernel_stack_end + 4096`). Only the latter is asserted — it is a
deliberate linker decision, whereas the former is incidental and would be a
fragile thing to pin.

### F. AGENTS.md compliance pass on §2.E

Stage 3 was audited against `AGENTS.md` and **four real violations were found and
fixed**. Recorded because the *pattern* is the useful part.

**1. Split-brain: the kernel table hardcoded where devices live.** It mapped MMIO
as one blanket 1 GiB gigapage, justified by a comment listing the QEMU virt
addresses — while `device_tree.rs` already had `uart_base/size`,
`plic_base/size`, `clint_base/size` parsed from the DTB. A second, coarser
encoding of knowledge the system already had, and `rw-` over 1 GiB to cover a few
MiB of registers.

> This is exactly the violation Stage 2 existed to remove — §2.D criticises
> `frame.rs` for "independently re-encoding boot.S's mapping decision" — and it was
> committed one commit later. The lesson is a question, not a rule: *does something
> already know this?*

Fixed by `device_tree::mmio_regions()`, now the single answer to "where is device
memory". The map went from one 1 GiB gigapage to exact windows:

```
uart    1 x 4KiB     plic  1536 x 4KiB     clint  16 x 4KiB
```

4 KiB is for *exactness*, not alignment — a superpage rounds outward, and next to
a device window sits either another device or nothing.

**2. DRY: a second range-mapping loop.** `kernel_table`'s `install()` duplicated
`Mapper::map_range`, generalised to any level. Fixed at the root: `paging` gained
`map_range_at_level`, `map_range` is now its level-0 wrapper, and the kernel's copy
is gone. A test asserts the two agree page-for-page, so collapsing them is proven
not to have changed behaviour. This also unblocks reuse — superpage ranges were
previously only available inside a private kernel module.

**3. Strict modularity: one 406-line file with five concerns.** Split into
`region.rs` (196 lines — the reusable *mechanism*: `Region`, `install`, `audit`,
`report`, generic over `FrameSource`/`PhysAccess`) and `kernel_table.rs` (295 —
the kernel's *policy* and the `satp` switch). A user address space can now reuse
the mechanism.

> Improvement that fell out of the split: validation moved **into**
> `Region::install`, so W^X and page alignment are enforced at the single choke
> point where a `Region` becomes PTEs. It used to be a separate pass over the list,
> which a future caller could simply forget to run.

**4. Latent split-brain: two owners for "which frames are reachable".** The table
mapped up to `device_tree::ram_end()` while `frame::init` clamped to
`direct_map::WINDOW_END` — the *boot* table's window, retired by then. They agreed,
but by parallel reasoning. Fixed by dependency inversion: `frame::owned_range()`
publishes the physical span the allocator took (bitmap included, since the
allocator's own `range()` excludes it), and the table maps exactly that. The
allocator decides what it will hand out; the table maps what the allocator owns.

**5. Minor: `rights()` re-spelled R/W/X in the kernel.** Moved to
`PteFlags::rwx()`, in the type that owns what those bits mean.

All three enforcement paths were then shown to be non-vacuous by mutation:

| Mutation | Caught by |
|---|---|
| `.text` given `RWX` | `region 'text' would be both writable and executable` |
| skip installing `rodata` | `region 'rodata' left 0xffffffc080218000 unmapped` |
| write to `.text` at run time | `scause 15` / `StorePageFault` |

### G. Stage 4 — DTB reserved, identity map retired

Two things, both boot/memory only.

**1. The device-tree blob is now withheld from the frame allocator.** It sits at
`0x87e00000 (size 0x17c4)` — squarely inside the pool `0x8032d000..0x88000000` —
so the allocator could hand out the pages the tree is stored in. §5 debt #1 for
three sessions; now closed.

> **A correction to an earlier claim in these notes.** The suggested cheap fix was
> "allocate over it at init and never free". That is **impossible**: `allocate(count)`
> returns whichever block happens to be free, never a chosen address. Nothing in the
> crate could claim a specific frame, so a real reservation primitive was the only
> option.

`FrameAllocator::reserve(range)` withholds an interior range. It is
`allocate`'s descent aimed at a *chosen* leaf: climb to the nearest ancestor that
is a whole free block, recording the path, then clear that ancestor and free every
sibling passed on the way down. `parent`/`sibling`/`block_at` already existed, so
the whole thing is ~20 lines.

- Reserved frames are indistinguishable from allocated ones, so reclaiming an
  initrd later is just `deallocate_at` — there is a test for exactly that.
- Frame-at-a-time at order 0. A largest-aligned-block walk would touch fewer
  nodes, but reservations are boot-time and small, and this is obviously correct
  where the clever version would need its own argument.
- Deliberately **not** unwound on failure: frames reserved before the failing one
  stay reserved, which is the safe direction.
- `device_tree::dtb_range()` supplies the extent from the FDT header's
  `totalsize`, rounded outward to whole frames. The size is printed in the boot log
  so the reservation can be checked against it: `0x17c4` rounds to 2 frames. ✓

**2. The identity mapping is gone.** The kernel table now maps nothing at
`VA == PA`; the entire low half of the address space is unmapped and available to
user processes. Three call sites had to stop treating a physical address as a
pointer:

| Site | Fix |
|---|---|
| `console.rs` | `MmioSerialPort::new(phys_to_virt(base))` — it caches the port in a `static` that outlives the boot table |
| `plic.rs` | 7 × `(plic_base() + OFFSET) as *mut u32` → one `register(offset)` accessor |
| `device_tree.rs` | `Fdt::from_ptr(phys_to_virt(dtb_ptr))` — the last one, and easy to miss |

The `plic.rs` change is worth noting as DRY rather than mechanical: seven copies of
one decision became one accessor, so there is a single place that knows how to
reach a PLIC register.

**This is Stage 2's payoff arriving.** `phys_to_virt` of a device address is only
usable because the direct map is *linear* — under the old RAM-base-skewed offset it
produced `0xffffffbf90000000`, not even a canonical Sv39 address. The same VA is
mapped by both `boot.S`'s table and the kernel table, so the conversions are valid
across the switch and the console never goes dark.

Verified empirically in both directions:

| Check | Result |
|---|---|
| console still prints after the switch | 125 lines of output, `tick 1` |
| raw physical UART read | `scause 13` / `LoadPageFault`, "probe failed" never printed |
| reservation actually withholds | mutating `reserve` to a no-op trips `reserving 2 device-tree frames did not remove them from the pool` |

### H. Stage 5 — the boot hart is not hart 0, and stacks have guard pages

Two items that were queued as "hardening for SMP that doesn't exist yet". One of
them turned out to be a **live, flaky bug**.

#### The boot hart is chosen by a lottery

`start()` gated one-time setup on `hartid == 0`. OpenSBI does not promise that: it
runs a lottery among the harts, so *which* hart enters the kernel varies from boot
to boot. Measured on the committed code at `-smp 4`, 8 runs:

```
run 1: boot hart=0  -> ok            run 5: boot hart=0  -> ok
run 2: boot hart=0  -> ok            run 6: boot hart=2  -> PANIC
run 3: boot hart=1  -> PANIC         run 7: boot hart=2  -> PANIC
run 4: boot hart=3  -> PANIC         run 8: boot hart=0  -> ok
```

`PANIC` is `device tree RAM region not discovered` — with `hartid != 0` the kernel
skipped `device_tree::init` entirely and then failed in `memory::init` for a
seemingly unrelated reason. **5 of 8 boots were broken.** It went unnoticed only
because `-smp 1` is what `scripts/run.sh` passes, and with one hart the lottery has
one entrant.

Fixed with `cpu::claim_boot_hart(hartid)` — a compare-exchange that exactly one
caller wins, so the role belongs to whoever actually arrived rather than to a hart
id guessed in advance. After: 8/8 pass, with harts 0, 1, 2 and 3 all observed
winning.

The boot hart is now printed at boot, because it is not a constant and a
hart-dependent failure should not be a mystery.

#### The same assumption had a second instance: the BSS

`boot.S` had `bnez a0, 3f` guarding the BSS-zeroing loop. So on every one of those
boots whose winner was not hart 0, **the BSS was never cleared at all**. It looked
fine only because QEMU hands out zeroed memory — it would not survive real hardware
or a warm restart.

Now claimed with `amoswap.w` on a flag in `.data` (it cannot live in `.bss`, which
is the thing being cleared), with `fence w,w` before publishing completion and a
spin-plus-`fence r,r` on the losing path so no hart can read a static before the
zeroing is visible. Correct whether one hart enters or many.

> The losing branch is **unexercised**: nothing calls SBI HSM `hart_start`, so only
> one hart ever enters. Verified by disassembly and by 8/8 boots across four
> different winners, not by racing it.

#### Per-hart stack guard pages

Stacks grow down, and `_bss_end` sat immediately below `_kernel_stack_start`, so
hart 0's overflow ran into `.bss` — and every other hart's into the previous hart's
stack. Each hart's stack now sits above its own unmapped guard page:

```
guard  [start + STRIDE*h,               start + STRIDE*h + GUARD_SIZE)
stack  [start + STRIDE*h + GUARD_SIZE,  start + STRIDE*(h+1))
```

so the stack top — and therefore `sp` — stays `start + STRIDE*(h+1)`, the same shape
`boot.S` already computed. The guards are holes because `kernel_table` maps each
stack as its own region and never the guards; `audit_holes` then checks all 16 are
genuinely unmapped, since "unmapped" is the one property invisible in a region list.

**Verified:** writing 8 bytes below hart 0's stack bottom now faults with
`scause 15` / `StorePageFault`. Before, that write silently landed in `.bss`.

`boot.S` also now derives the hart limit and parks anything beyond it, instead of
letting an out-of-range hartid compute an `sp` inside someone else's stack.

#### Where the geometry lives, and why that way round

The obvious approach — declare the sizes in `kernel.ld` and read them from Rust —
**does not link**:

```
relocation R_RISCV_PCREL_HI20 out of range: 66584062 is not in [-524288, 524287];
references '_hart_guard_size'
```

`layout.rs` reads a linker symbol by taking its address, which is PC-relative. That
works for `_text_start` because its value is near the code; it cannot work for a
*size* like 4096. (`li t0, _sym` in asm fails for the same underlying reason:
"operand must be a constant 64-bit integer".)

So it is inverted: `kernel.ld` reserves only the **total**, and `memory::stack` owns
the subdivision and **derives** `max_harts` from that total. Growing the area grows
the hart count; nothing can hold a stale copy. `boot.S` derives the same number the
same way, reading only `STRIDE` from a word `memory::stack` exports — the pattern
already used for `EARLY_SATP_TEMPLATE`.

#### Also removed

- `arch::NCPU = 8` — a dead constant with no users and a **third**, disagreeing
  answer to "how many harts" (the linker reserves 16). Deleted, with a pointer to
  `stack::max_harts()`.
- The second copy of the `csrw satp; sfence.vma` sequence, which `install()` would
  have duplicated. Both paths now go through one `switch_to`.

#### Secondary-hart path

`kernel_table::init` publishes the `satp` last, so it doubles as the barrier
`install()` waits on: non-zero means frames, heap and table are all up.
`memory::init_secondary()` adopts the table rather than rebuilding it, and every
hart's stack is already mapped so the switch is safe from any hart with a reserved
stack. Still unreachable — no `hart_start` caller — but the split is now explicit
rather than a comment.

### I. AGENTS.md compliance pass on §2.H — the magic number

§2.H claimed the stack geometry had "exactly one definition". It did not. The linker
script reserved the area with a bare literal:

```
. = . + 0x110000;   /* memory::stack divides this into 16 x (4 KiB + 64 KiB) */
```

That is a **composite** magic number: `0x110000` silently encodes all three of
`MAX_HARTS`, `GUARD_SIZE` and `SIZE`. Deriving `max_harts` in Rust made the two
*consistent* but not single-sourced — and arguably made it worse, because the number
became unverifiable. Change `stack::SIZE` to 32 KiB and `max_harts()` would quietly
become `0x110000 / 0x9000 = 18`, with 8 KiB wasted and the comment now a lie. The
comment itself restated a subdivision the linker did not own.

**Fixed by inverting who declares the size.** `memory::stack` declares the whole
area as one static:

```rust
#[used]
#[unsafe(link_section = ".hart_stacks")]
static HART_STACKS: HartStacks = HartStacks(UnsafeCell::new([0; STRIDE * MAX_HARTS]));
```

and `kernel.ld` merely *places* it, taking the size from the section:

```
.hart_stacks (NOLOAD) : {
    PROVIDE(_kernel_stack_start = .);
    KEEP(*(.hart_stacks))
    PROVIDE(_kernel_stack_end = .);
} :bss
```

No size appears in the linker script at all. `NOLOAD` keeps the 1 MiB out of the
image (verified: `.hart_stacks` is `NOBITS`, and the flat image stayed 168 KiB);
`KEEP` is required because nothing in Rust references the static, so `--gc-sections`
would otherwise discard the kernel's stacks. `UnsafeCell` because the bytes are
written by hardware via `sp`, never through the item.

Proven non-vacuous by mutation: setting `stack::SIZE = 32 * 1024` resized the section
to `0x90000` on its own and the boot log reported `16 x 32 KiB` — no stale total
anywhere.

**The page size, too.** `4096` appeared eight times in `kernel.ld`. Now a single
`_page_size` symbol. It still cannot be shared with Rust's `PAGE_SIZE` (same
PC-relative limitation), so `memory::layout::check()` pins them together at boot —
not by reading the symbol, but by measuring something the linker *built* with it: the
gap it left between the stacks and the heap must equal `PAGE_SIZE`. It also asserts
every separately-mapped section starts on a page. Mutation-tested: setting
`_page_size = 8192` trips

```
kernel.ld padded 0x2000 bytes between the stacks and the heap, but Rust's
PAGE_SIZE is 0x1000; the linker's _page_size and PAGE_SIZE disagree
```

**And one more coupled constant:** `MAX_REGIONS = 48` was a guess that happened to
be big enough for 16 harts. Now `stack::MAX_HARTS + 16`, so raising the hart count
cannot silently overflow the region list.

The `_page_size` rename was verified to move nothing: stack span, heap guard and all
section alignments are byte-identical before and after (the absolute addresses shift
by one page only because the new assertions grew `.text`).

Remaining literals in `kernel.ld` are the four documented inputs — `_page_size`
(architectural) plus `_dram_base`, `_va_offset`, `_text_offset` (platform facts) —
and `ALIGN(16)`, the sub-page alignment inside a section.

### J. Stage 6 — reservations are enumerable, and firmware carve-outs reach them

`reserve()` could withhold memory but nothing recorded *what*. A reserved frame and
an allocated frame are indistinguishable in the bitmap — that is what makes
reclaiming an initrd a plain `deallocate_at`, but it also meant nothing could answer
"why is this memory not free?", and a 200-frame leak looked exactly like a 200-frame
firmware carve-out.

**The latent bug it was hiding.** §2.I fixed `mmio_regions()` by classifying
`/reserved-memory` as *not* a device — correctly, since OpenSBI's PMP denies S-mode
access to it. But the ranges were then **discarded**: discovered and dropped. On QEMU
virt they sit at `0x80000000..0x80050000`, *below* the kernel image, while the pool
starts at `0x80346000`, so they miss it and are safe **by accident**. Firmware
reserving memory *above* the kernel is entirely normal, and then the allocator would
have vended it.

That is the same failure shape as the earlier ones: a fact correctly established and
then not routed to the thing that needed it.

**Now:**

- `device_tree` does **one** walk producing two lists — MMIO windows and
  reserved-memory ranges — through a single `classify()`, so the two cannot disagree
  about which node is which. `MmioRegion` became `PhysRegion`, since both lists are
  the same shape and having two identical types would be the duplication again.
- `frame` keeps a `RESERVATIONS` list; `reserve(name, start, end)` records every
  withholding and the boot log prints **from the record**, not from a `println!` at
  each call site.
- Both sources are fed in: the blob and every `/reserved-memory` range.
- A range that misses the pool is *reported*, not silently ignored — "outside the
  pool" and "forgot to reserve" must not look the same:

```
[dtb] mmio:  17 windows, 2 reserved ranges (from one walk of the tree)
[memory] reserve: mmode_resv1@80000000 at 0x80000000..0x80040000 is outside the pool, skipped
[memory] reserve: mmode_resv0@80040000 at 0x80040000..0x80050000 is outside the pool, skipped
[memory] withheld 2 frames in 1 reservations:
[memory]   device tree blob         0x87e00000..0x87e02000 (8 KiB)
```

**Verified for the case this platform never produces.** Injecting a synthetic
carve-out inside the pool gives `withheld 5 frames in 2 reservations` with both named,
and `reserve`'s internal `free_frames` assertion passes — so a carve-out that *does*
land in the pool is withheld and recorded, which is the whole reason the code exists.

### K. Stage 7 — the DTB was not the only foreign RAM

Asking "is the device-tree blob the only thing inside the pool that isn't ours?"
turned up **four** sources, of which two were unhandled and one was a live bug.

The previous boot stage has four different ways of leaving something in RAM, and
honouring some of them is indistinguishable from honouring none:

| Source | Where | Was it handled? |
|---|---|---|
| `/reserved-memory` nodes | tree nodes | yes (§2.J) |
| The blob itself | `0x87e00000` | yes (§2.J) |
| **FDT memory reservation block** | header `off_mem_rsvmap` | **no** |
| **initrd**, `/chosen linux,initrd-start/end` | `0x84200000` | **no — live bug** |

**The initrd is the live one.** `fdt_raw`'s `Chosen` does not expose it, so nothing
read it. Measured with `-initrd`:

```
[PROBE] linux,initrd-start: len=8 u64=Some(2216689664)   = 0x84200000
[PROBE] linux,initrd-end:   len=8 u64=Some(2250244096)   = 0x86200000
```

The pool is `0x8034a000..0x88000000`, so a 32 MiB initrd lands **squarely in the
middle of it** and the allocator would hand it out. `rfs` and `blockdev` are already
in the tree; the first attempt to mount a root filesystem from an initrd would have
hit this.

**The reservation block is the standards one.** The FDT spec has *two* mechanisms for
reserved memory — the `/reserved-memory` node and the header-level reservation block —
and reading only the former honours half the standard. Empty on QEMU virt with
OpenSBI (measured: 0 entries), so latent here, but U-Boot and coreboot populate it.

**Fixed by widening the concept rather than adding two more special cases.**
`reserved_memory()` became `foreign_ram()`: one list, four sources, each entry named
by where it came from — so `frame` iterates exactly one thing, and a later reclaim
(an initrd is finished with once the root filesystem is mounted) can find its range
by name. `dtb_range()` is gone, since the blob is now just another entry.

Verified both ways:

```
# no initrd
[dtb] mmio:  17 windows, 3 foreign RAM ranges (from one pass over the tree)
[memory] withheld 2 frames in 1 reservations:
[memory]   device tree blob         0x87e00000..0x87e02000 (8 KiB)

# -initrd hdd.dsk
[dtb] mmio:  17 windows, 4 foreign RAM ranges (from one pass over the tree)
[memory] withheld 8194 frames in 2 reservations:
[memory]   device tree blob         0x87e00000..0x87e02000 (8 KiB)
[memory]   initrd                   0x84200000..0x86200000 (32 MiB)
```

8194 = 2 + 8192, and `reserve`'s `free_frames` assertion passed for both, so the
32 MiB is genuinely withheld rather than merely logged.

> The lesson is the question, not the fix. "Is the DTB reserved?" was the wrong
> question; "what did the previous boot stage leave in RAM that the pool's *start
> address* does not already exclude?" is the right one, and it has four answers on
> this platform.

### L. Stage 8 — SMP actually runs, and it found two bugs immediately

`hart_start` was the last item, and its only purpose was to make already-written
code *executable*. It did that, and the first successful boot found two bugs in
code I had previously called "verified by disassembly".

**Added:** an SBI v0.2 call form (`a6` = FID, `a7` = EID, `(error, value)` returned in
`a0`/`a1`) — the legacy form can't express it — plus HSM `hart_start` and
`hart_get_status`, and `device_tree::hart_ids()`.

Two details that matter:

- **`start_addr` must be physical.** SBI starts a hart with `satp = 0`, so the entry
  cannot be a Rust function at a high VA — the first instruction fetch would fault.
  It is `virt_to_phys(text_start())`, and each secondary walks the whole `boot.S`
  path itself: early table, jump high, then diverge at `claim_boot_hart`.
- **Hart ids are not a count.** `/cpus/cpu@N`'s `reg` *is* the hart id, and real
  platforms leave gaps. `hart_ids()` returns the list; `stack::max_harts()` is a
  different fact (how many we have stacks for) and `start_secondaries` is where the
  two meet.

#### Bug 1: `amoswap` cannot express a conditional claim

The BSS claim used `amoswap.w` with 1. That writes 1 **whatever it finds**. A hart
arriving after the flag reached 2 (done) clobbered it back to 1, then span forever
waiting for a 2 that could never return. **All three secondaries deadlocked in
`boot.S`** — started successfully per SBI, never printing a byte.

Fixed with `lr.w`/`sc.w`, which stores only if nothing touched the word since the
load, so a hart that observes 2 never writes at all.

> §2.H said of this exact code: *"The losing branch is unexercised … verified by
> disassembly and by 8/8 boots across four different winners, not by racing it."*
> The caveat was correct and the code was still wrong. Disassembly confirms what the
> instructions *are*, never what they *mean* under contention.

#### Bug 2: the lock-free console writer shreds output

`trap_handler` printed two lines per trap through `kprintln!`, which bypasses the UART
lock. With one hart, invisible. With eight taking timer interrupts, the console became
character-level garbage:

```
 c[hartod 0] [te: 5,r asep_pc: 0xffhaffndlefr]f scausc0e c80o20de2afc:
```

`kmain`'s `kprintln!` had the same effect on secondaries announcing themselves.

The rule the code now follows: **`kprintln!` is only for contexts where this hart may
already hold the lock** — a panic, or an exception taken inside `_print` itself.
Not interrupts (`_print` masks them, so the locked path is safe), and not ordinary
logging. Routine per-trap logging is gone entirely; `sepc` is reported on the
*exception* arm only, where it is worth having and rare enough not to interleave.

Also replaced `kmain_ap`'s placeholder tight `ebreak` loop with `wait_forever()`. It
was harmless only because no secondary had ever reached it; the first one to arrive
would have trapped on every iteration and buried the console.

#### Verified

| Config | Result |
|---|---|
| `-smp 2` | 6/6 |
| `-smp 4` | 6/6 |
| `-smp 8` | 6/6 |

"Correct" meaning every hart appears, exactly `n-1` reach `kmain_ap`, and no panic or
unexpected exception. Boot hart varied across 0, 1, 2, 3, 5, 6, 7 over these runs.

**Paths that now genuinely execute for the first time:** `boot.S`'s BSS wait branch,
`memory::init_secondary` → `kernel_table::install`, per-hart stacks and their guard
pages on more than one hart, and `claim_boot_hart`'s compare-exchange with real
contention.

### M. AGENTS.md compliance pass on §2.L

Auditing the SMP commit found three violations, **two of which that commit
introduced** — one of them a straight regression of the bug it had just fixed.

**1. The console fix was a comment, not a fix.** Rule 1 is *don't paper over a
problem at the symptom site*, and that is precisely what happened: two call sites
corrected, a paragraph written, the trap left in place. The root cause was the
**name**:

```
println!      locked, safe
kprintln!     lock-free, shreds concurrent output
```

One letter apart, reading like a drop-in. That name alone was enough for the same
mistake to be made twice independently — `trap_handler` and `kmain` — each looking
reasonable in review.

Renamed to `emergency_print!` / `emergency_println!`, long and alarming on purpose:
`emergency_println!("enter kmain")` does not survive a second look, where
`kprintln!("enter kmain")` did. `_kprint` → `_emergency_print`, and the misleading
`KernelStdout` sink → `SbiConsole`. It now has exactly three users, all in the
panic/abort handler — its only reason to exist.

**2. The fix re-introduced the same bug one arm over.** §2.L removed routine logging
from the interrupt arm and put this on the exception arm:

```rust
Trap::Exception(e) => {
    kprintln!("[trap] exception {e:?} at sepc {epc:#x}");
    exceptions::handler(e, tf)
}
```

But `exceptions::handler` dispatches `UserEnvCall` — **every system call**. So the
moment syscalls exist, each one prints through the lock-free writer: the identical
failure mode, moved from timer interrupts to syscalls. It was also redundant, since
the catch-all `panic!` already named the exception.

`epc` is now threaded into `exceptions::handler` and reported *in* that panic:

```
Aborting: file crates/kernel/src/trap/exceptions/mod.rs:23:
        unexpected exception: StorePageFault at sepc 0xffffffc0802061d8
```

One message, on the fatal path only. `ecall.rs` moved to the locked `println!` — an
`ecall` is executed deliberately, so it can never arrive while this hart is inside
`_print`, which is the only thing the emergency path is for.

**3. `print_info` reported memory layout from inside `cpu/`.** It imported nothing but
`memory::layout` and `memory::stack` — a CPU module reporting another subsystem's
business. Split: `memory::report_layout()` prints the image layout and stack geometry
(called from `memory::init`, which owns those symbols), and `cpu::print_info()` keeps
only CPU identity.

> And while doing that I added a duplicate: `cpu::print_info` printed the hart list
> that `device_tree::summary` already prints. Caught in the boot log on the next run
> and removed. Fixing DRY violations is apparently a good way to create one.

#### Considered and rejected

- `start_secondaries` checking `hart >= max_harts` while `boot.S` also parks such a
  hart — different responsibilities: `boot.S` must guard *any* entry, including one
  the firmware initiates, while `start_secondaries` merely declines to invite one.
  Both derive from the same constant, pinned by `check_layout`.
- `MAX_MMIO` / `MAX_FOREIGN` / `MAX_HART_IDS` fixed capacities — `device_tree::init`
  runs before the heap exists, so these cannot be `Vec`s the way `MAX_REGIONS` could.
- `sbi_call` vs `sbi_call_ext` — genuinely different ABIs.

#### Flagged, not fixed (pre-existing)

All four legacy SBI IPI/fence wrappers pass `&hart_mask as *const _ as usize` — a
**virtual stack address** where the firmware expects a pointer. All unused, but IPIs
are the natural next reach from HSM, so the landmine now sits next to live code.

Verified: `-smp` 1/2/4/8, 5 runs each, 20/20.

---

## 3. Verified state

```
cargo test -p paging --features std     # 43 passed  (NOTE: --features std is required;
                                        #  without it the crate is no_std → 0 tests run)
cargo test -p frame-allocator           # 25 passed
cargo build -p paging                   # no_std, host        ) both, to keep the
cargo build -p paging --target riscv64imac-unknown-none-elf   ) crate honest
cargo kbuild                            # builds; 34 warnings, all pre-existing
cargo krun                              # boots to kmain
```

Boot log — `direct map:` is the Stage 2 line, and `frames:` still precedes
`heap:` (the §2.B ordering fix). Note the heap VA is now `0xffffffc0_80800000`:
PA `0x80800000` + `VA_OFFSET`, i.e. the linear map, where it used to be
`0xffffffc0_00800000`.

```
[memory] direct map: PA 0x0..0x100000000 -> VA 0xffffffc000000000.. (4 GiB)
boot hart: 0 (chosen by the firmware, not assumed)
[dtb] blob at 0x87e00000 (size 0x17c4)
[memory] reserved device tree: 0x87e00000..0x87e02000 (2 frames)
[memory] frames: 0x8032d000..0x88000000 (124 MiB, physical)
[memory] frame allocator self-test passed
[memory] heap:   0xffffffc080800000..0xffffffc081000000 (8 MiB, virtual)
[memory] kernel page table root at 0x87e02000:
[memory]   uart                   0xffffffc010000000 -> 0x0010000000  rw-     1 x 4KiB
[memory]   plic                   0xffffffc00c000000 -> 0x000c000000  rw-  1536 x 4KiB
[memory]   clint                  0xffffffc002000000 -> 0x0002000000  rw-    16 x 4KiB
[memory]   text                   0xffffffc080200000 -> 0x0080200000  r-x    26 x 4KiB
[memory]   rodata                 0xffffffc08021a000 -> 0x008021a000  r--    14 x 4KiB
[memory]   data                   0xffffffc080229000 -> 0x0080229000  rw-     2 x 4KiB
[memory]   bss                    0xffffffc08022b000 -> 0x008022b000  rw-     2 x 4KiB
[memory]   hart stacks            0xffffffc08022e000 -> 0x008022e000  rw-   256 x 4KiB (x16)
[memory]   frame pool head        0xffffffc08033e000 -> 0x008033e000  rw-   194 x 4KiB
[memory]   direct map             0xffffffc080400000 -> 0x0080400000  rw-    62 x 2MiB
[memory] kernel page table live (satp 0x8000000000087e02); boot table retired
enter kmain
[timer] tick 1
```

`hart stacks (x16)` is sixteen regions collapsed into one line by `region::report`; they are separate regions precisely so the guard page between each pair stays unmapped. The `direct map tail` region is absent because this platform's RAM top is already
superpage-aligned, so it is empty and skipped. `tick 1` is the proof traps still
work *after* the switch.

ELF facts verified by inspection after the relink (`llvm-readelf`, `llvm-nm`,
and a byte-level dump of the table out of the image):

```
.text     VMA ffffffc080200000   LMA 80200000   (uniform skew = VA_OFFSET)
.rodata   VMA ffffffc080214000   PROGBITS, in the first LOAD segment
EARLY_PGTABLE        ffffffc080218000  R   (page-aligned, .rodata)
EARLY_SATP_TEMPLATE  ffffffc080219000  R   = 0x8000000000000000
_va_offset           ffffffc000000000  A

EARLY_PGTABLE: exactly 8 non-zero entries of 512 —
  root[0..3]     -> PA 0, 1G, 2G, 3G   flags 0xcf (V R W X A D)
  root[256..259] -> PA 0, 1G, 2G, 3G   flags 0xcf
table PA 0x80218000  ->  boot.S writes satp = 0x8000000000080218
```

`0xcf` is bit-for-bit what the old `ori t3, t3, 0xcf` produced — the encoding
did not change, only *who* computes it.

Environment facts established by inspection:

- Target is `code-model: medium` (medany) + `relocation-model: static` → symbol
  refs are PC-relative and VMA−LMA is a uniform constant across the image. This
  is what makes `lla` yield *physical* addresses pre-paging, which is how
  `boot.S` finds `EARLY_PGTABLE`. Stage 2 no longer *runs* Rust pre-paging, so
  the two hazards that would have mattered (relaxation to `gp`-relative,
  absolute relocations in read data) are moot; verified anyway — the whole
  pre-`satp` path is `auipc`/`addi` and the image has **no relocations at all**.
- QEMU runs `-m 128M`, RAM base `0x8000_0000` → RAM is `0x80000000..0x88000000`,
  comfortably inside the 4 GiB direct-map window.
- `_va_offset = 0xffffffc000000000` (pure constant); kernel links at
  `0xffffffc080200000`, loads at `0x80200000`.
- `VPN[2]` of `0xffffffc000000000` is **256**, so the direct map occupies root
  entries 256..259 and the kernel image itself sits in **258**
  (`0xffffffc080200000`). All four high-half VAs are canonical Sv39; the old
  skewed offset's MMIO VA was not.

---

## 4. OPEN — remaining boot & memory work

The boot/memory path is now essentially complete: PMM before heap, a const boot
table over a linear direct map, a W^X kernel table with no identity mapping, and
the DTB withheld. What is left in this area is small and mostly about *scale*
rather than correctness.

### 4.1 SMP — mostly closed, one piece left

The reachable half is fixed (§2.H): the boot hart is claimed rather than assumed,
the BSS is claimed rather than gated on hart 0, and secondary harts have an
`init_secondary` that adopts the kernel table instead of rebuilding it.

What remains is that **nothing starts a secondary hart**. `sbi.rs` implements only
legacy SBI v0.1 — there is no HSM `hart_start`, so `init_secondary`, `install`, and
`boot.S`'s BSS wait-branch are all correct-by-construction but unexercised. Bringing
APs up means adding the HSM extension, and only then can those paths be tested by
racing them rather than by inspection.

Note `-smp 4` now boots reliably, which it did not before — but that is one hart
running, chosen from four, not four harts running.

### 4.2 `GLOBAL` on kernel mappings

Deliberately omitted so far. Kernel regions live in every address space, so marking
their leaves `G` lets the TLB keep them across an ASID switch. It is pure
optimisation and it has no observable effect until there is more than one address
space — which is why it was deferred, and why it should land *with* user paging
rather than before it.

### 4.3 RAM above the direct-map window

`direct_map::WINDOW_GIGAPAGES = 4` bounds what the *boot* table reaches, and
`frame::init` clamps to it, warning loudly about anything above. Only bites on a
machine with more than 4 GiB below the window; lifting it is a one-line change plus
a check that the const table still fits (512 root entries, 8 in use).

Note the kernel table has no such bound — it maps exactly `frame::owned_range()` —
so this is purely a boot-phase limit.

### 4.4 Nice-to-haves, in rough order of value

- **Superpages for aligned device windows.** QEMU virt's PLIC is 3 aligned MiB
  mapped as 1536 4 KiB leaves. Picking the largest level that divides both base and
  size would cut that to 3, at the cost of a size-must-divide argument.
- **A reservation list rather than one-shot reserve.** `FrameAllocator::reserve`
  handles the DTB; an initrd or DTB `/reserved-memory` nodes would each need
  another call, which is fine, but nothing currently *enumerates* what was reserved.
- **~34 dead-code warnings** in `plic`/`utils`/`trap`/`proc`. Unrelated to memory;
  they have survived four stages untouched.

### 4.5 Explicitly out of scope here

User processes, per-process page tables, `U=1` pages, the syscall path, and the VMA
/ address-space manager (§1's third column, still unowned). All of it is queued
behind the memory work, and all of it is where `FrameSource::free` /
`frame::free_at` finally get a caller — they still have **zero** today, which makes
that the least-exercised code in the subsystem.


## 5. Known debt (flagged, not silently ignored)

Struck-through items are closed; kept so the history of each is legible.

1. ~~DTB not reserved.~~ **Done** (§2.G) — `FrameAllocator::reserve` withholds
   `0x87e00000..0x87e02000`. Note the earlier suggested fix ("allocate over it and
   never free") was **impossible**: `allocate` cannot target an address.
2. **RAM above the direct-map window is dropped.** Warned loudly, never silently
   truncated. Bounds the *boot* table only — the kernel table maps exactly
   `frame::owned_range()`. One constant, `direct_map::WINDOW_GIGAPAGES`. §4.3.
3. ~~No free-by-PFN.~~ **Done** — `deallocate_at` (§2.E). Residual sharp edge: the
   *order* passed cannot be validated, so it is `unsafe` and the token-based
   `deallocate` stays the default. `frame::free_at` hardcodes order 0, correct for
   every caller it documents accepting.
4. **`proc/mod.rs` is entirely commented out** against the old heap-allocated
   `Table` API. Out of scope for the memory work (§4.5).
5. ~~`Mapper` has no kernel adopter.~~ **Done** — `memory/kernel_table.rs` (§2.E).
6. ~~The kernel runs on the boot table.~~ **Done** — W^X kernel table, verified by a
   `StorePageFault` probe against `.text` (§2.E).
7. ~~Identity mappings survive.~~ **Done** (§2.G) — the kernel table maps nothing at
   `VA == PA`; the whole low half is unmapped. Narrowed three times before dying:
   boot table's low 4 GiB `RWX` → one `rw-` gigabyte → exact DTB windows → gone.
   Verified by a `LoadPageFault` probe on a raw physical address.
8. **`FrameSource::free` and `frame::free_at` have no caller.** Correct and tested
   at the crate level, but the kernel table is permanent so nothing tears down a
   tree. The least-exercised code in the subsystem; user paging (§4.5) is what will
   first run it.
9. ~~Single-hart only.~~ **Done** (§2.H, §2.L). The boot hart is claimed rather than
   assumed, and secondaries are started via SBI HSM. Verified at `-smp` 2/4/8, 6 runs
   each. Doing so immediately exposed two bugs in code previously "verified by
   disassembly" — an `amoswap` that could not express a conditional claim, and a
   lock-free console writer that shredded concurrent output.
10. **No `GLOBAL` bit on kernel mappings** — TLB optimisation, deferred to when
    address spaces exist (§4.2).
11. ~~No guard page below the kernel stack.~~ **Done** (§2.H) — one unmapped guard
    page per hart, all 16 audited, verified by a `StorePageFault` probe 8 bytes
    below hart 0's stack bottom.
12. ~~Reservations are not enumerable.~~ **Done** (§2.J) — `frame::reservations()`
    records every withholding, the boot log prints from it, and `/reserved-memory` is
    now fed in rather than discovered and discarded.
13. **~34 pre-existing dead-code warnings** in `plic`/`utils`/`trap`/`proc`.
    Unrelated to memory; the count has not moved across four stages.


---

## 6. Commands

```bash
cargo test -p paging --features std   # 43 — the --features std is NOT optional
cargo test -p frame-allocator         # 25
cargo kbuild                          # build kernel (riscv64imac-unknown-none-elf)
cargo krun                            # boot under QEMU + OpenSBI (Ctrl-A X to exit)

# capture a boot log non-interactively:
timeout 20 cargo krun < /dev/null > /tmp/kboot.log 2>&1
grep -aE '\[memory\]|self-test|Aborting|enter kmain' /tmp/kboot.log
```

Verifying the boot mapping without booting — the const table is in the image, so
it can be read straight out of the ELF. This is how §3's table dump was produced,
and it is the fastest way to check a change to `direct_map`:

```bash
ELF=target/riscv64imac-unknown-none-elf/debug/kernel
llvm-nm "$ELF" | grep -E 'EARLY_PGTABLE|EARLY_SATP|_va_offset'
llvm-readelf --section-headers "$ELF" | grep -E '\.text|\.rodata'   # VMA vs LMA skew
llvm-readelf -r "$ELF"                                             # must be: no relocations
llvm-objdump -d --start-address=0xffffffc080200040 \
             --stop-address=0xffffffc080200088 "$ELF"               # pre-paging path
```

Probing whether a protection actually holds — the boot log stating `r-x` is not
proof the hardware agrees. Inject a temporary write into `kmain`, expect
`scause 15` / `StorePageFault`, then revert and re-check the disassembly:

```rust
// TEMP: writing to .text must fault once the kernel table is live.
unsafe { core::ptr::write_volatile(crate::memory::layout::text_start() as *mut u8, 0u8) };
kprintln!("PROBE FAILED: the write to .text succeeded");
```

Same trick verifies the `direct_map::verify` guard: add `addi a2, a2, 8` after
`boot.S`'s `sub a2, t1, t0` to corrupt the measurement without touching the
mapping. Always confirm the revert by disassembly, not by reading the source.

Relevant files: `crates/kernel/src/boot.S`, `crates/kernel/kernel.ld`,
`crates/kernel/src/memory/{mod,direct_map,kernel_table,region,frame,layout}.rs`,
`crates/paging/src/satp.rs`, `crates/paging/src/sv39/*`,
`crates/frame-allocator/src/*`.
