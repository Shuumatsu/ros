# Parked: the trap subsystem

This directory is **not compiled**. It is `crates/kernel/src/trap/` as it stood when
the boot and memory-init path was still being finalised, moved out of the crate
whole rather than commented out in place. `git log --follow` on any file here still
works.

## Why it left

Traps are out of scope until boot and memory init are done. Keeping the module
compiled-but-uninvoked would have meant maintaining it against an API that is still
moving, and would have implied a level of correctness it does not have — see the
defects below. Commenting out the bodies would have been worse: that is what version
control is for.

Nothing was lost in the move. The architecture boot entry points every hart's `stvec`
at a parking vector before ordinary Rust runs, then re-points it at the high alias.
So a trap in the current kernel stops the faulting hart deterministically with
`scause`/`sepc`/`stval` intact. For a boot path with no console guarantees that is
strictly more useful than a half-built dispatcher, because every trap in this phase
is a bug.

## What was actually live

Only the timer. Everything else here was already dead code that the compiler could
not see, because it sat behind entry points nobody called:

- `interrupts/plic.rs` — `plic::init()` was never called (there is a `TODO` where the
  call should be). `sie::set_sext()` was therefore never set, so `SupervisorExternal`
  could not fire and `plic::handler` was unreachable.
- `interrupts/clint/mod.rs` — `clint::init()` had no callers; `interrupts::init()`
  reached past it straight to `clint::timer::init()`. So `software::init()` never
  ran, `sie::set_ssoft()` was never set, and `SupervisorSoft` would have hit the
  `unimplemented!()` arm in `interrupts::handler` had anything sent one.
- `context.rs` — **does not compile.** It refers to `crate::cpu::regs::GeneralRegs`,
  and there is no `cpu::regs` module. This is why `trap/mod.rs` carried a
  `// mod context;` line rather than a real one.

## Before any of this comes back

Do not restore it wholesale — it needs a rewrite, not a `git mv` in the other
direction. In particular:

1. **`context.rs` needs `cpu::regs`, or needs deleting.** It overlaps `TrapFrame`
   heavily; decide whether user context and trap frame are one type or two before
   writing either. Two types that both mean "saved registers" is the split-brain the
   project standards forbid.
2. **Decide the PLIC/CLINT ownership question.** We are an S-mode payload: the M-mode
   CLINT belongs to the SBI firmware, which is why `clint/timer.rs` goes through
   `sbi::set_timer` and does not touch CLINT MMIO. `clint/` is therefore a misleading
   name for what is really "SBI TIME extension". Rename on the way back in.
3. **`sbi::set_timer` no longer exists — bring it back with the timer.** It was a
   one-line `sbi_call(0, ...)` whose only caller was `interrupts/clint/timer.rs` in
   here, so it was deleted along with the blanket `#![allow(dead_code)]` that had
   been hiding it and two others. Restoring it means re-adding the wrapper and the
   `SBI_SET_TIMER: usize = 0` constant to `arch/riscv64/sbi.rs`.
4. **`SupervisorSoft` must be handled before the SBI IPI/RFENCE wrappers get written.**
   `arch/riscv64/sbi.rs` documents why those five legacy wrappers were deleted and why
   their replacements are deliberately unwritten; they land with their first caller.
5. **Re-check `memory::kernel_table::switch_to`.** It masks interrupts across the
   `satp` write and TLB flush. That mask is currently protecting against nothing,
   since no source is enabled. It must still be correct the moment a timer can fire
   during memory bring-up.
6. **`start.rs` calls nothing.** `stvec` is a CSR, so whatever replaces the parking
   vector is per-hart and both Rust entries need it.
