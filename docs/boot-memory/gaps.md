# Gaps

What the boot memory path does not do. Listed because a gap nobody wrote down reads as a
gap nobody found.

## Unexercised, not unwritten

| what | state |
|---|---|
| `kernel_table::with` | no callers. The documented way in for anything that maps after boot has never run on hardware. |
| `Mapper::unmap`, `Mapper::free_subtables` | host tests only. Nothing in the kernel tears a mapping down yet. |
| frame reclaim | `FrameAllocator::deallocate_at` supports handing a reserved range back, so the device-tree blob and an initrd are reclaimable. Nothing does it. |

## Deliberate, with a cost

| what | why, and what it costs |
|---|---|
| cross-hart TLB shootdown | `sfence.vma` is not a broadcast. Editing a tree live on another hart needs an IPI and the RFENCE SBI extension. Nothing edits a live tree from a second hart yet, so nothing is wrong today. |
| `kernel_va` is bump-only | Every consumer so far is permanent, and a watermark cannot fragment or double-vend. Anything temporary needs a free list, and it belongs in that module. |
| the heap never shrinks | Frames become heap for good. The backstop is a ceiling — the smaller of a fixed maximum and a share of the pool — not a return path. |
| `PteFlags::GLOBAL` unset | A TLB optimisation whose correctness depends on address spaces that do not exist yet. |
| `Frames` has no `Drop` | Releasing needs the allocator lock, so a dropped token leaks. `#[must_use]` and `leak()` make the intent explicit instead. |
| one RAM bank | Only the `/memory` bank containing the kernel is managed. Others are reported, not dropped silently. |
| `/reserved-memory`'s `no-map` | A carve-out is withheld from the frame allocator, not excluded from the direct map, so one landing inside the frame pool is mapped although the property forbids it. On QEMU virt both of OpenSBI's sit below the kernel image, outside the pool. |

## Bounded by construction

| bound | beyond it |
|---|---|
| direct map reaches 128 GiB of physical address space | RAM above it is dropped with a warning; a *device* window above it is fatal at `MachineMemory::check`, which names the constant to raise |
| `MAX_MMIO`, `MAX_FOREIGN`, `MAX_HART_IDS` | fixed capacities. Overflow warns and names the constant; it does not truncate quietly. |
| `MAX_FOREIGN` also bounds overlap detection | past it, `frame::reserve` warns that overlap detection is incomplete — the frames stay withheld either way |

## Known-wrong

`device_tree` uses a `reg` address as a CPU physical address directly, which is silently
wrong on a board whose `/soc` declares a non-identity `ranges`. Its module doc owns the
detail, including why composing `Fdt::translate_address` with `Node::path` is not
straightforward.

## Testing

`mmu`, `frame-allocator` and `heap` are host-tested. The kernel crate is not: it is
`no_std`, `no_main` and forced to the RISC-V target, so everything in `memory` — the region
layout, the audits, the ordering — is exercised only by booting.

That is a real hole. Confirming any one of those audits means injecting the fault it exists
to catch and reading the panic off a QEMU boot, which `verify.md` gives as a procedure.
Closing the hole means moving whatever is pure geometry out of the kernel crate and behind a
host-testable boundary, the way `mmu` already is.
