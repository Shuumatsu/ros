//! Per-hart state and secondary-hart startup.
//!
//! Physical hart IDs are sparse. Dense CPU indices alone index storage, with slot zero
//! reserved for the firmware-selected boot hart.

use core::mem::offset_of;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use mmu::VirtualAddr;
use spin::Once;

use crate::arch::{self, boot};
use crate::memory::kernel_table;
use crate::memory::stack::{self, Stack};
use crate::println;
use crate::time;

/// Per-hart control block addressed through `tp` and, during trap entry, `sscratch`.
///
/// Relaxed accesses occur before publication or on the owning hart.
#[repr(C)]
pub struct Cpu {
    hartid: AtomicUsize,
    index: AtomicUsize,
    ticks: AtomicU64,
    /// Process kernel-stack top, or zero while no process is running.
    kernel_stack_top: AtomicUsize,
    /// Scratch word used exclusively by trap-entry assembly.
    trap_spill: AtomicUsize,
    /// Running process control-block pointer, or zero.
    process: AtomicUsize,
}

impl Cpu {
    /// Field offsets consumed by trap-entry assembly.
    pub const TRAP_SPILL: usize = offset_of!(Cpu, trap_spill);
    pub const KERNEL_STACK_TOP: usize = offset_of!(Cpu, kernel_stack_top);
    // Plain assembly accesses are confined to this block's owning hart during trap entry.

    const fn new() -> Self {
        Self {
            hartid: AtomicUsize::new(0),
            index: AtomicUsize::new(0),
            ticks: AtomicU64::new(0),
            kernel_stack_top: AtomicUsize::new(0),
            trap_spill: AtomicUsize::new(0),
            process: AtomicUsize::new(0),
        }
    }

    pub fn hartid(&self) -> usize { self.hartid.load(Ordering::Relaxed) }

    pub fn index(&self) -> usize { self.index.load(Ordering::Relaxed) }

    pub fn record_tick(&self) -> u64 {
        let ticks = self.ticks.load(Ordering::Relaxed) + 1;
        self.ticks.store(ticks, Ordering::Relaxed);
        ticks
    }

    /// Installs a process control block and kernel stack as one invariant.
    ///
    /// # Panics
    ///
    /// Panics if this hart is already running a process.
    pub fn enter_process(&self, control_block: usize, kernel_stack_top: VirtualAddr) {
        assert_eq!(
            self.process.load(Ordering::Relaxed),
            0,
            "hart {} is already running a process; the second would take the first's stack",
            self.hartid()
        );
        self.kernel_stack_top.store(kernel_stack_top.bits(), Ordering::Relaxed);
        self.process.store(control_block, Ordering::Relaxed);
    }

    pub fn leave_process(&self) {
        self.process.store(0, Ordering::Relaxed);
        self.kernel_stack_top.store(0, Ordering::Relaxed);
    }

    pub fn process(&self) -> Option<usize> {
        Some(self.process.load(Ordering::Relaxed)).filter(|&block| block != 0)
    }
}

/// Cache-line-isolated storage for one hart.
#[repr(C, align(64))]
struct CpuSlot {
    cpu: Cpu,
    handoff: boot::SecondaryHandoff,
    stack: Once<Stack>,
}

impl CpuSlot {
    const fn new() -> Self {
        Self { cpu: Cpu::new(), handoff: boot::SecondaryHandoff::new(), stack: Once::new() }
    }
}

/// Maximum number of harts the kernel can run.
pub const MAX_CPUS: usize = 64;

const _: () = assert!(
    align_of::<CpuSlot>() == arch::CACHE_LINE_BYTES,
    "a cpu slot must be aligned to a whole cache line, or two harts share one"
);

static CPU_SLOTS: [CpuSlot; MAX_CPUS] = [const { CpuSlot::new() }; MAX_CPUS];
static BOOT_READY: AtomicBool = AtomicBool::new(false);

fn try_current() -> Option<&'static Cpu> {
    let tp = arch::thread_pointer();
    // SAFETY: nonzero `tp` is installed from the static CPU slot array.
    (tp != 0).then(|| unsafe { &*(tp as *const Cpu) })
}

/// This hart's control block.
///
/// # Panics
///
/// Panics before this hart adopts a CPU.
pub fn current() -> &'static Cpu {
    try_current().expect("tp is null: the boot entry did not adopt a Cpu")
}

/// Returns this hart's physical ID, or `None` before CPU adoption.
pub fn try_hart_id() -> Option<usize> { try_current().map(Cpu::hartid) }

static ONLINE: AtomicUsize = AtomicUsize::new(0);

fn install_current(cpu: &'static Cpu) {
    // SAFETY: `cpu` is a permanent control block with the representation `tp` readers expect.
    unsafe { arch::adopt_control_block(cpu as *const Cpu as usize) };
}

pub fn init_boot(hartid: usize) {
    let cpu = &CPU_SLOTS[0].cpu;
    cpu.hartid.store(hartid, Ordering::Relaxed);
    cpu.index.store(0, Ordering::Relaxed);
    install_current(cpu);

    assert!(!BOOT_READY.swap(true, Ordering::AcqRel), "boot Cpu initialized twice");
}

/// # Safety
///
/// `cpu_pointer` must point to the assigned [`Cpu`] within [`CPU_SLOTS`].
pub unsafe fn init_secondary(hartid: usize, cpu_pointer: usize) {
    let cpu = unsafe { &*(cpu_pointer as *const Cpu) };
    install_current(cpu);
    assert_eq!(
        hartid,
        cpu.hartid(),
        "hart id disagreement: SBI entered hart {hartid}, handoff selected hart {}",
        cpu.hartid()
    );
}

pub fn record_online() { ONLINE.fetch_add(1, Ordering::Release); }

pub fn boot_hart() -> Option<usize> {
    BOOT_READY.load(Ordering::Acquire).then(|| CPU_SLOTS[0].cpu.hartid())
}

/// Every hart the machine reports except the boot hart, capped at the cpu slots left.
///
/// # Panics
///
/// Panics before [`init_boot`].
fn secondary_hart_ids() -> impl Iterator<Item = usize> {
    const SLOTS: usize = MAX_CPUS - 1;

    let boot = boot_hart().expect("secondary_hart_ids called before the boot hart was recorded");
    let reported = crate::device_tree::hart_ids();
    let wanted = reported.iter().filter(|&&hart| hart != boot).count();
    if wanted > SLOTS {
        println!(
            "[smp] WARNING: the machine reports {wanted} harts besides the boot hart and this \
             kernel has {SLOTS} cpu slots; the rest are left stopped"
        );
    }
    reported.iter().copied().filter(move |&hart| hart != boot).take(SLOTS)
}

/// Number of initialized secondary slots; initialization distinguishes zero from not assigned.
static SECONDARIES: Once<usize> = Once::new();

/// Claim a cpu slot and a kernel stack for every hart this kernel means to start.
///
/// Call once after allocator initialization and before building the kernel page table.
///
/// # Panics
///
/// Panics if called twice.
pub fn assign_stacks() {
    assert!(SECONDARIES.get().is_none(), "cpu::assign_stacks called twice; the slots are filled");

    let mut filled = 0;
    for (index, hart) in (1..).zip(secondary_hart_ids()) {
        assert!(index < MAX_CPUS, "cpu slot {index} is out of range; secondary_hart_ids caps it");
        let slot = &CPU_SLOTS[index];
        slot.cpu.index.store(index, Ordering::Relaxed);
        slot.cpu.hartid.store(hart, Ordering::Relaxed);
        slot.stack.call_once(|| stack::alloc("secondary stack"));
        filled += 1;
    }

    SECONDARIES.call_once(|| filled);
}

/// Bring up every hart [`assign_stacks`] claimed a slot for.
///
/// Call once from the boot hart after the kernel page table is installed.
pub fn start_secondaries() {
    let satp = kernel_table::satp()
        .expect("no kernel page table published; start_secondaries ran before memory");
    let secondaries = *SECONDARIES
        .get()
        .expect("no cpu slots assigned; start_secondaries ran before cpu::assign_stacks");
    let entry = boot::entry_address();

    let mut requested = 0;
    for slot in &CPU_SLOTS[1..=secondaries] {
        let (hart, index) = (slot.cpu.hartid(), slot.cpu.index());
        let stack = *slot.stack.get().expect("assign_stacks gives every slot it fills a stack");
        let cpu = &slot.cpu as *const Cpu as usize;

        match boot::start_cpu(hart, &slot.handoff, satp, stack.top(), cpu) {
            Ok(()) => {
                requested += 1;
                println!(
                    "[smp] started hart {hart} (cpu {index}) at {entry:#x}, stack top {:#x}",
                    stack.top()
                )
            }
            Err(error @ boot::StartError::NotStopped(_)) => {
                println!("[smp] hart {hart} (cpu {index}) not started: {error}")
            }
            Err(error) => {
                println!("[smp] WARNING: hart {hart} (cpu {index}) not started: {error}")
            }
        }
    }

    await_secondaries(requested);
}

/// Waits for accepted starts using a counter deadline; skips the wait without a timebase.
fn await_secondaries(requested: usize) {
    const TIMEOUT_SECS: u64 = 2;

    if requested == 0 {
        return;
    }

    let Some(deadline) = time::deadline(TIMEOUT_SECS) else {
        println!(
            "[smp] no /cpus/timebase-frequency; not waiting for the {requested} secondaries \
             (their arrival is still logged individually)"
        );
        return;
    };
    if time::spin_until(deadline, || ONLINE.load(Ordering::Acquire) >= requested) {
        return;
    }

    let online = ONLINE.load(Ordering::Acquire);
    println!(
        "[smp] WARNING: {} of {requested} secondaries never reached the kernel after \
         {TIMEOUT_SECS}s; inspect sepc/scause/stval",
        requested - online
    );
}

pub fn print_info() {
    match boot_hart() {
        Some(hart) => println!("boot hart: {hart} (chosen by the firmware)"),
        None => println!("boot hart: unrecorded"),
    }
}
