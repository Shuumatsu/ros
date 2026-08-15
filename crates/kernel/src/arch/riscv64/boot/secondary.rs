//! Secondary-hart startup: the state the boot hart publishes, and the prologue that
//! consumes it.
//!
//! Together because they are one interface written twice — the `#[repr(C)]` struct, and
//! the field offsets the assembly loads from it.

use core::sync::atomic::{AtomicUsize, Ordering};

const PUBLISHED: usize = 1;

/// Data published by the boot hart before an SBI HSM start.
#[repr(C)]
pub(crate) struct SecondaryHandoff {
    ready: AtomicUsize,
    satp: AtomicUsize,
    stack_top: AtomicUsize,
    cpu: AtomicUsize,
}

impl SecondaryHandoff {
    pub(crate) const fn new() -> Self {
        Self {
            ready: AtomicUsize::new(0),
            satp: AtomicUsize::new(0),
            stack_top: AtomicUsize::new(0),
            cpu: AtomicUsize::new(0),
        }
    }

    pub(crate) fn publish(&self, satp: usize, stack_top: usize, cpu: usize) {
        assert_eq!(self.ready.load(Ordering::Relaxed), 0, "secondary handoff published twice");
        assert_ne!(satp, 0, "secondary handoff needs a live page table");
        assert_ne!(stack_top, 0, "secondary handoff needs a stack");
        assert_ne!(cpu, 0, "secondary handoff needs a Cpu");

        self.satp.store(satp, Ordering::Relaxed);
        self.stack_top.store(stack_top, Ordering::Relaxed);
        self.cpu.store(cpu, Ordering::Relaxed);
        self.ready.store(PUBLISHED, Ordering::Release);
    }
}

const READY_OFFSET: usize = core::mem::offset_of!(SecondaryHandoff, ready);
const SATP_OFFSET: usize = core::mem::offset_of!(SecondaryHandoff, satp);
const STACK_TOP_OFFSET: usize = core::mem::offset_of!(SecondaryHandoff, stack_top);
const CPU_OFFSET: usize = core::mem::offset_of!(SecondaryHandoff, cpu);

/// Adopt the kernel page table, the stack the boot hart reserved and the [`Cpu`]
/// it chose, then enter Rust.
///
/// Still assembly, because the stack is only mapped by the kernel table: `sp` cannot be
/// set before the switch, and no Rust runs before `sp`.
///
/// Reached from [`super::entry::enter_high`] with `a0` the hart id and `a1` the `opaque`
/// from `hart_start` — this hart's handoff, as a kernel VA, already reachable because the
/// boot table maps the high half too.
///
/// [`Cpu`]: crate::cpu::Cpu
#[unsafe(naked)]
#[unsafe(link_section = ".text.init.entry")]
pub(super) unsafe extern "custom" fn prologue() {
    boot_asm!({
        // Does not spin in practice — `publish` finishes before `hart_start` — but it is
        // the acquire half of that release store: SBI does not promise the start request
        // orders the boot hart's writes, and every field below is garbage if nothing does.
        "1:",
        "ld    t0, {ready}(a1)",
        "beqz  t0, 1b",
        "fence r, rw",
        // Read out before the switch. `a1` is about to be reused for the argument,
        // and the table it points through is about to be replaced.
        "ld    t0, {satp}(a1)",
        "ld    t1, {stack_top}(a1)",
        "ld    a1, {cpu}(a1)",
        "csrw  satp, t0",
        "sfence.vma",
        // Before the `tail`, which expands through `t1`.
        "mv    sp, t1",
        "tail  {secondary}",
    }
        ready = const READY_OFFSET,
        satp = const SATP_OFFSET,
        stack_top = const STACK_TOP_OFFSET,
        cpu = const CPU_OFFSET,
        secondary = sym crate::start::secondary,
    )
}
