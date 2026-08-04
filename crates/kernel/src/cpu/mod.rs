use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use paging::MemoryAddr;

use crate::arch::riscv64::{boot, sbi};
use crate::memory::{kernel_table, stack, virt_to_phys};
use crate::println;

/// Per-hart control block. `tp` points at this hart's.
///
/// Following Linux, which keeps a pointer here rather than a bare id: `tp` is the
/// one register that is per-hart for free, so spending it on an integer we were
/// already handed in `a0` means every future piece of per-hart state needs a new
/// home and a new lookup.
///
#[repr(C)]
pub struct Cpu {
    /// Physical hart id from the SBI boot protocol. Sparse — never an array index.
    hartid: AtomicUsize,
    /// Dense logical index, `0..cpus`. This is the array subscript; `hartid` is not.
    index: AtomicUsize,
}

impl Cpu {
    const fn new() -> Self { Self { hartid: AtomicUsize::new(0), index: AtomicUsize::new(0) } }

    /// Physical hart id, for SBI calls and diagnostics.
    pub fn hartid(&self) -> usize { self.hartid.load(Ordering::Relaxed) }

    /// Dense logical index, for anything array-shaped.
    pub fn index(&self) -> usize { self.index.load(Ordering::Relaxed) }
}

struct CpuSlot {
    cpu: Cpu,
    handoff: boot::SecondaryHandoff,
}

impl CpuSlot {
    const fn new() -> Self { Self { cpu: Cpu::new(), handoff: boot::SecondaryHandoff::new() } }
}

/// Upper bound on harts this kernel will run.
const MAX_CPUS: usize = 64;

/// Slot 0 belongs to the firmware-selected boot hart.
static CPU_SLOTS: [CpuSlot; MAX_CPUS] = [const { CpuSlot::new() }; MAX_CPUS];
static BOOT_READY: AtomicBool = AtomicBool::new(false);

/// This hart's control block.
///
/// # Panics
/// If `tp` is null, which means the Rust entry did not adopt a CPU first.
pub fn current() -> &'static Cpu {
    let tp: usize;
    // SAFETY: reading a register.
    unsafe { core::arch::asm!("mv {}, tp", out(reg) tp, options(nomem, nostack)) };
    assert!(tp != 0, "tp is null: the boot entry did not adopt a Cpu");
    // SAFETY: `install_current` only accepts pointers into the static CPU slot array.
    unsafe { &*(tp as *const Cpu) }
}

/// This hart's physical id. Every `[hart N]` console prefix comes from here.
pub fn hart_id() -> usize { current().hartid() }

/// Secondary harts that have reached [`crate::start::secondary`].
static ONLINE: AtomicUsize = AtomicUsize::new(0);

fn install_current(cpu: &'static Cpu) {
    let pointer = cpu as *const Cpu as usize;
    // SAFETY: `tp` is reserved for the kernel's per-hart pointer.
    unsafe { core::arch::asm!("mv tp, {}", in(reg) pointer, options(nomem, nostack)) };
}

/// Initialize slot 0 and make it current before diagnostics can panic.
pub fn init_boot(hartid: usize) {
    let cpu = &CPU_SLOTS[0].cpu;
    cpu.hartid.store(hartid, Ordering::Relaxed);
    cpu.index.store(0, Ordering::Relaxed);
    install_current(cpu);

    assert!(!BOOT_READY.swap(true, Ordering::AcqRel), "boot Cpu initialized twice");
}

/// Adopt the CPU selected by the boot hart for this secondary.
///
/// # Safety
///
/// `cpu_pointer` must point at a `Cpu` in [`CPU_SLOTS`].
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

/// Record that a secondary hart made it into Rust, on the kernel page table.
pub fn record_online() { ONLINE.fetch_add(1, Ordering::Release); }

/// The firmware-selected boot hart.
pub fn boot_hart() -> Option<usize> {
    BOOT_READY.load(Ordering::Acquire).then(|| CPU_SLOTS[0].cpu.hartid())
}

/// Every hart the machine reports except the boot hart.
///
/// Defined against [`boot_hart`] rather than against whoever happens to be asking, so
/// the answer does not depend on which hart calls it. Iterated, never ranged over:
/// hart ids need not be `0..n`.
///
/// Consumed exactly once, by [`crate::memory::init`], which allocates a stack per
/// entry and stores the pairing. [`start_secondaries`] then reads that pairing back
/// instead of walking this again — one traversal, so there is no second answer to
/// disagree with the first.
///
/// # Panics
/// Before [`init_boot`], since "every hart except the boot hart" has no
/// meaning yet.
pub fn secondary_hart_ids() -> impl Iterator<Item = usize> {
    let boot = boot_hart().expect("secondary_hart_ids called before the boot hart was recorded");
    crate::device_tree::hart_ids().iter().copied().filter(move |&hart| hart != boot)
}

/// Bring up every hart [`crate::memory::init`] reserved a stack for.
///
/// Call once, from the boot hart, **after** [`crate::memory::init`]: it hands each
/// hart a stack that only the kernel page table maps, so both have to exist first.
///
/// # What the hart is told
///
/// Two things, and it derives nothing:
///
/// - `start_addr` is the physical address of the architecture's stackless secondary
///   entry.
/// - `opaque`, which lands in `a1`, points at one release-published handoff containing
///   the final page table, stack top, and prepared [`Cpu`].
pub fn start_secondaries() {
    let entry = virt_to_phys(boot::secondary_entry_address());
    let satp = kernel_table::satp()
        .expect("no kernel page table published; start_secondaries ran before memory::init");

    // Slot 0 belongs to the boot hart, so secondaries start at 1.
    let mut requested = 0;
    for (index, &stack::Secondary { hart, stack }) in (1..).zip(stack::secondaries()) {
        assert!(index < MAX_CPUS, "machine reports more than {MAX_CPUS} harts; raise MAX_CPUS");
        let slot = &CPU_SLOTS[index];
        // Ask first. "Already started" and "no such hart" are different problems, and
        // a bare error code from hart_start would not distinguish them.
        match sbi::hart_get_status(hart) {
            Ok(sbi::HartState::Stopped) => {}
            Ok(state) => {
                println!("[smp] hart {hart} not started: firmware reports {state:?}");
                continue;
            }
            Err(error) => {
                println!("[smp] hart {hart} status unavailable: {error}");
                continue;
            }
        }

        assert!(
            stack.top().is_aligned(16),
            "hart {hart}'s stack top {:#x} is not 16-byte aligned",
            stack.top()
        );

        slot.cpu.index.store(index, Ordering::Relaxed);
        slot.cpu.hartid.store(hart, Ordering::Relaxed);
        slot.handoff.publish(satp, stack.top().bits(), &slot.cpu as *const Cpu as usize);

        let opaque = &slot.handoff as *const boot::SecondaryHandoff as usize;
        match sbi::hart_start(hart, entry.bits(), opaque) {
            Ok(()) => {
                requested += 1;
                println!(
                    "[smp] started hart {hart} (cpu {index}) at {entry:#x}, stack top {:#x}",
                    stack.top()
                )
            }
            Err(error) => println!("[smp] hart {hart} failed to start: {error}"),
        }
    }

    await_secondaries(requested);
}

/// Wait for the harts we asked for to actually arrive, and say so if they do not.
///
/// `hart_start` returning `Ok` means only that the firmware *accepted* the request —
/// the hart is `StartPending`. Without confirming arrival, a secondary that faults
/// inside the stackless entry (a bad stack mapping, say) parks forever while
/// the boot hart continues and `kmain` prints its success line: a boot with N-1 dead
/// harts is indistinguishable from a good one.
///
/// # The bound is a duration, not a spin count
///
/// A spin count measures the *host's* speed, not the guest's: a number generous on
/// real hardware can exceed a minute under QEMU's TCG interpreter, making the
/// timeout indistinguishable from the hang it exists to report.
///
/// `rdtime` reads the `time` CSR — readable in S-mode because OpenSBI sets
/// `[m|s]counteren.TM` — and `/cpus/timebase-frequency` says what its ticks are
/// worth, so the bound is a real second anywhere. This needs no timer *interrupt*,
/// only a CSR read, so it does not depend on the parked trap subsystem.
///
/// Without the frequency there is no clock to bound by, so the wait is skipped and
/// said so rather than guessed at.
fn await_secondaries(requested: usize) {
    /// Long enough that a slow emulated hart is not slandered, short enough that a
    /// genuinely dead one does not look like a hang.
    const TIMEOUT_SECS: u64 = 2;

    if requested == 0 {
        return;
    }

    let Some(hz) = crate::device_tree::timebase_hz() else {
        println!(
            "[smp] no /cpus/timebase-frequency; not waiting for the {requested} secondaries \
             (their arrival is still logged individually)"
        );
        return;
    };

    let deadline = now() + TIMEOUT_SECS * hz as u64;
    while now() < deadline {
        if ONLINE.load(Ordering::Acquire) >= requested {
            return;
        }
        core::hint::spin_loop();
    }

    // Not a panic. Losing a secondary is bad but not fatal to the boot hart, and a
    // kernel that can still report the loss is more useful than one that stops.
    let online = ONLINE.load(Ordering::Acquire);
    println!(
        "[smp] WARNING: {} of {requested} secondaries never reached the kernel after \
         {TIMEOUT_SECS}s; inspect sepc/scause/stval",
        requested - online
    );
}

/// The `time` CSR: a free-running counter, readable in S-mode.
#[inline]
fn now() -> u64 {
    let t: u64;
    // SAFETY: `rdtime` is a plain CSR read with no side effects.
    unsafe { core::arch::asm!("rdtime {}", out(reg) t, options(nomem, nostack)) };
    t
}

/// Report what this hart is, as opposed to what memory looks like.
///
/// The kernel image layout used to be printed here too, which put a function that
/// imports nothing but `memory::layout` and `memory::stack` in the CPU module. It
/// lives in [`crate::memory::report_layout`] now; this reports CPU facts only.
pub fn print_info() {
    match boot_hart() {
        Some(hart) => println!("boot hart: {hart} (chosen by the firmware, not assumed)"),
        None => println!("boot hart: unrecorded"),
    }
}
