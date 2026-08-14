# Contracts

The boundaries the boot path crosses, and the facts that exist twice because a boundary
would not carry them. Each duplicate is named here with the check that pins it, because no
single file can hold both halves.

## What firmware hands over

`a0` is this hart's id. `a1` is the device tree on the boot hart, or `hart_start`'s
`opaque` on a secondary. `satp` is zero and `sstatus.SIE` is clear.

**Every other register is undefined**, `sp`, `gp` and `tp` included. That is why the entry
is assembly: no compiled function can be called before there is a stack, and no absolute
link-time address — a jump table, a vtable, a `&'static str` — resolves before translation
is on.

## Facts stated twice

| fact | first copy | second copy | pinned by |
|---|---|---|---|
| direct-map base | `kernel.ld` `_va_offset` | `memory::direct_map::VA_OFFSET` | `direct_map::verify`, against the skew `enter_high` measures at its high-half jump |
| page size | `kernel.ld` `_page_size` | `paging::sv39::PAGE_SIZE` | `memory::layout::check`, by measuring the guard gap the linker actually built |
| boot stack geometry | `kernel.ld` places `.boot_stack` | `memory::stack::STRIDE` declares its size | `memory::stack::check_layout` |
| `satp` encoding | `paging::Satp::sv39` | `boot_table::SATP_TEMPLATE` + `SATP_ROOT_SHIFT`, which the entry's `srli`+`or` reassembles | a `const` assert in `memory::boot_table`, one root per PPN bit |
| handoff field offsets | `SecondaryHandoff`'s `#[repr(C)]` fields | the offsets the prologue's assembly loads | `offset_of!`, so the assembly cannot drift from the struct |
| which sections exist | `kernel.ld`'s `SECTIONS` | the ranges `kernel_table` maps | `ASSERT`s at the foot of `kernel.ld`, at link time |

The pattern is the same every time: state it twice because the two languages cannot share a
value, then measure what was really built rather than trusting the copy.

## Typed seams

| boundary | crossed as | so that |
|---|---|---|
| platform → `memory` | `memory::machine::MachineMemory` | `memory` reads no device tree; a board without an FDT means another builder and no change here |
| `memory` → `paging` | `FrameSource` (`address_space::TableFrames`) and `PhysAccess` (`LinearOffset(VA_OFFSET)`) | `paging` never allocates and never assumes a physical address is a pointer |
| `heap` → frames | `heap::Outcome::Grow { at_least }`, returned rather than fetched | the heap's lock is released before the frame allocator's is taken |
| boot hart → secondary | `SecondaryHandoff`, release-published and acquire-read | SBI does not promise the start request orders the boot hart's writes |
| page table → hardware | `AddressSpace::edit` / `walk`, never a bare `Mapper` | every write is followed by a fence; Sv39 caches the *absence* of a translation too |

## Lock order

One direction only: the heap's lock is never held while the frame allocator's is taken.
`heap::GrowableHeap::allocate` returns "I need this much" instead of calling out, which is
what makes the order a property of the type rather than a promise in a comment.

Both locks are `sync::IrqMutex`, not `spin::Mutex`: a `#[global_allocator]` and a frame
allocator are both reachable from a trap handler, and a handler waiting on a lock its own
hart holds waits forever.

## Audits before the `satp` switch

All of these run in `kernel_table::init` while the boot table is still live, because a
mis-mapped `.text` faults on the instruction *after* `csrw satp`, with the old table gone.

| check | catches |
|---|---|
| `region::audit_disjoint` | two regions sharing a page, which means sharing a PTE |
| `kernel_table::audit_kernel_va` | a mapping above the direct map that no allocator handed out |
| `Region::audit`, every page | wrong level, wrong rights, wrong frame |
| `kernel_table::audit_holes` | a guard page that is mapped, which is a guard that guards nothing |
| `kernel_table::audit_live_context` | the running PC and `sp`, read from the machine, not surviving the switch |
| `Region::validate` | W^X violations, and a superpage region whose rounding would pull in a neighbour |

`region` holds the ones that are mechanism, so a user address space gets them over its own
list; `kernel_table` holds the ones that know the kernel's layout.
