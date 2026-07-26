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
[dtb] blob at 0x87e00000 (size 0x17c4)
[memory] reserved device tree: 0x87e00000..0x87e02000 (2 frames)
[memory] frames: 0x8032d000..0x88000000 (124 MiB, physical)
[memory] frame allocator self-test passed
[memory] heap:   0xffffffc080800000..0xffffffc081000000 (8 MiB, virtual)
[memory] kernel page table root at 0x8032f000:
[memory]   uart                   0xffffffc010000000 -> 0x0010000000  rw-     1 x 4KiB
[memory]   plic                   0xffffffc00c000000 -> 0x000c000000  rw-  1536 x 4KiB
[memory]   clint                  0xffffffc002000000 -> 0x0002000000  rw-    16 x 4KiB
[memory]   text                   0xffffffc080200000 -> 0x0080200000  r-x    26 x 4KiB
[memory]   rodata                 0xffffffc08021a000 -> 0x008021a000  r--    14 x 4KiB
[memory]   data                   0xffffffc080228000 -> 0x0080228000  rw-     2 x 4KiB
[memory]   bss                    0xffffffc08022a000 -> 0x008022a000  rw-     2 x 4KiB
[memory]   kernel stack           0xffffffc08022c000 -> 0x008022c000  rw-   256 x 4KiB
[memory]   frame pool head        0xffffffc08032d000 -> 0x008032d000  rw-   211 x 4KiB
[memory]   direct map             0xffffffc080400000 -> 0x0080400000  rw-    62 x 2MiB
[memory] kernel page table live (satp 0x800000000008032f); boot table retired
enter kmain
[timer] tick 1
```

The `direct map tail` region is absent because this platform's RAM top is already
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

### 4.1 SMP — the one real correctness gap left

`memory::init` and `kernel_table::init` are called by **every** hart that reaches
`start`. At `-smp 1` (what `scripts/run.sh` passes) that is fine; beyond it, every
hart would re-initialise the frame allocator over the same RAM and build its own
kernel page table.

The fix has a shape already: hart 0 builds and publishes the `Satp` value;
secondaries skip straight to installing it. `Satp` is `Copy` and `bits()` is all an
AP needs, so a `static AtomicUsize` plus an `install(satp)` is close to sufficient.
Note `device_tree::init` is already correctly guarded to hart 0 — only the memory
path is not.

Worth doing **before** user processes, not after: retrofitting per-hart state into a
larger surface is strictly harder.

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

- **A guard page below the kernel stack.** The stack/heap guard above it exists and
  is asserted, but `_bss_end` and `_kernel_stack_start` are adjacent, so stack
  *overflow* runs into `.bss` silently. Needs a `. = . + 4096` in `kernel.ld`.
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
9. **Single-hart only.** `memory::init` and `kernel_table::init` run on every hart
   that reaches `start` — fine at `-smp 1`, wrong beyond it. The one real
   correctness gap left in this area; see §4.1.
10. **No `GLOBAL` bit on kernel mappings** — TLB optimisation, deferred to when
    address spaces exist (§4.2).
11. **No guard page below the kernel stack.** `_bss_end` and `_kernel_stack_start`
    are adjacent, so stack overflow lands in `.bss` silently. The guard *above* the
    stack exists and is asserted. Needs a linker-script change (§4.4).
12. **Reservations are not enumerable.** `reserve` works, but nothing records what
    was withheld, so a future initrd or `/reserved-memory` pass has no list to
    consult (§4.4).
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
