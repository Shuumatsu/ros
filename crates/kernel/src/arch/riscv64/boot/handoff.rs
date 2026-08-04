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

pub(super) const READY_OFFSET: usize = core::mem::offset_of!(SecondaryHandoff, ready);
pub(super) const SATP_OFFSET: usize = core::mem::offset_of!(SecondaryHandoff, satp);
pub(super) const STACK_TOP_OFFSET: usize = core::mem::offset_of!(SecondaryHandoff, stack_top);
pub(super) const CPU_OFFSET: usize = core::mem::offset_of!(SecondaryHandoff, cpu);
