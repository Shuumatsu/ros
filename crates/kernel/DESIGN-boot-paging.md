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

---

## 3. Verified state

```
cargo test -p paging --features std     # 39 passed  (NOTE: --features std is required;
                                        #  without it the crate is no_std → 0 tests run)
cargo test -p frame-allocator           # 13 passed
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
[hart 0] initializing memory...
[hart 0] [memory] direct map: PA 0x0..0x100000000 -> VA 0xffffffc000000000.. (4 GiB)
[hart 0] [memory] frames: 0x80325000..0x88000000 (124 MiB, physical)
[hart 0] [memory] frame allocator self-test passed
[hart 0] [memory] heap:   0xffffffc080800000..0xffffffc081000000 (8 MiB, virtual)
[hart 0] enter kmain
```

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

## 4. OPEN — Stage 3: a real kernel page table, and a `Mapper` adopter

Stage 2 removed the split-brain and gave the kernel a correct, uniform direct
map. What it did **not** do is refine the mapping: the kernel still runs on the
boot table, where everything is a 1 GiB `RWX` gigapage.

### 4.1 The remaining Stage-2 item, deliberately deferred

The previous handoff listed **"a kernel `FrameSource` impl wrapping
`frame::alloc()`"** as needed in Stage 2. It was skipped on purpose, and the
reason should not be forgotten:

`FrameSource` has two methods. `alloc_zeroed` is a one-liner over
`frame::alloc()`. `free(frame: PhysicalAddr)` **cannot be implemented at all**
today — `frame::free` consumes a move-only `Frames` token and `frame-allocator`
has no free-by-address (§5 debt #3). So the impl would ship with a panicking
hole, and nothing in the kernel constructs a `Mapper` to call it.

Writing it now buys a dead API with a landmine in it. It lands in Stage 3
together with `deallocate_at` on the crate and its first real caller.

### 4.2 What Stage 3 actually needs

1. **`deallocate_at(frame, order)` on `frame-allocator`.** Needed to free a page
   whose only remaining handle is its PTE. Note a buddy bitmap *can* recover the
   order by scanning up from 0, but for page-table frames the order is always 0,
   so the narrow version is enough to start.
2. **A kernel `FrameSource`** over `frame::alloc()` / `deallocate_at`, plus a
   `PhysAccess` — which is now trivially `LinearOffset(direct_map::VA_OFFSET)`,
   exactly the impl `paging` already ships. (Note Stage 1 built `LinearOffset`
   for a skewed offset that could not have worked for MMIO; Stage 2's linear map
   is what makes it correct.)
3. **Build the refined kernel table** with `Mapper`: `.text` R+X, `.rodata` R,
   `.data`/`.bss`/stack R+W, the direct map R+W (no X), and 4 KiB or 2 MiB leaves
   instead of one blanket gigapage. All the linker symbols are already exposed by
   `memory/layout.rs`.
4. **Switch `satp`** to it — `Satp::sv39` exists now, so this is a `csrw` from
   Rust plus an `sfence.vma`, no asm.
5. **Then drop the identity map.** This is the payoff Stage 2 unlocked: with a
   linear map, `phys_to_virt` is valid for MMIO, so nothing needs raw physical
   addresses as pointers any more. Today `console.rs` still uses the device-tree
   UART base directly, which only works because of the identity half — that is
   the one call site to convert first.

Sequencing note: 3 and 4 must be one atomic step. Installing a table that
mis-maps `.text` faults on the instruction after `csrw satp`, so the refined
table wants a `translate()` self-check over each section *before* it goes live —
`Mapper` already has `translate`.


## 5. Known debt (flagged, not silently ignored)

1. **DTB not reserved.** `frame::init`'s managed range can still span the
   device-tree blob. Pre-existing (the old buddy allocator had the same gap);
   there is a `TODO` in `frame::init`. Harmless today because nothing re-reads the
   raw DTB after `device_tree::init`, but it must be reserved before that changes.
2. **RAM above the direct-map window is dropped.** Warned loudly, never silently
   truncated. The bound is now one constant — `direct_map::WINDOW_GIGAPAGES` — so
   lifting it is a one-line change plus a re-check that the table still fits (it
   does: 512 root entries, 8 in use).
3. **No free-by-PFN.** `frame-allocator` frees via a move-only `FrameBlock` token.
   Reclaiming a mapped page from just its PTE will need a
   `deallocate_at(frame, order)` method on the crate. Today's only consumers
   (permanent heap + self-test) don't need it; **it is now the blocker for the
   kernel `FrameSource` impl** — see §4.1.
4. **`proc/mod.rs` is entirely commented out** and references the old
   heap-allocated `Table` API. It must be rewritten against `Mapper` when process
   support lands.
5. **`Mapper` still has no kernel adopter.** Stage 1 built the API; Stage 2 needed
   only `Table::map_gigapage`, which is deliberately *below* `Mapper`. First
   adopter is Stage 3 (§4.2).
6. **The kernel still runs on the boot table** — everything is a 1 GiB `RWX`
   gigapage, so `.text` is writable and `.rodata` is executable. No W^X, no
   guard pages. This is the headline reason Stage 3 exists.
7. **The identity map is still installed and still needed**, because `console.rs`
   uses the raw device-tree UART base as a pointer. Stage 2 made it *possible* to
   drop (`phys_to_virt` is valid for MMIO now); actually dropping it is Stage 3.5.
8. **~34 pre-existing dead-code warnings** in `plic`/`utils`/`trap`/`proc`.
   Untouched, unrelated — the count did not move across Stage 2.

---

## 6. Commands

```bash
cargo test -p paging --features std   # 39 — the --features std is NOT optional
cargo test -p frame-allocator         # 13
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

Relevant files: `crates/kernel/src/boot.S`, `crates/kernel/kernel.ld`,
`crates/kernel/src/memory/{mod,direct_map,frame,layout}.rs`,
`crates/paging/src/satp.rs`, `crates/paging/src/sv39/*`,
`crates/frame-allocator/src/*`.
