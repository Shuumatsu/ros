//! Which harts exist, which one is running this code, and bringing the rest up.
//!
//! Hart identity lives here rather than in the ISA layer: `tp` points at this hart's
//! [`Cpu`], and reading a field out of it is this module's business.
//!
//! Two numbering schemes, deliberately distinct. A *hart id* is the platform's, sparse
//! and firmware-chosen; a *cpu index* is dense, `0..cpus`, assigned here, and the only
//! one that subscripts anything. Slot 0 is the boot hart.
//!
//! [`start_secondaries`] tells a starting hart two things and lets it derive nothing:
//! the stackless entry to begin at, and one release-published handoff carrying the page
//! table, stack top and prepared [`Cpu`]. Everything it needs is therefore chosen by a
//! hart that already has a heap and a page table.

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use paging::MemoryAddr;

use crate::arch::riscv64::{boot, sbi};
use crate::memory::{kernel_table, stack, virt_to_phys};
use crate::println;
use crate::time;

/// Per-hart control block. `tp` points at this hart's.
///
/// A pointer rather than a bare id, as Linux does: `tp` is the one register that is
/// per-hart for free, and spending it on an integer already handed to us in `a0` would
/// leave every future piece of per-hart state needing a home and a lookup.
#[repr(C)]
pub struct Cpu {
    /// Physical hart id from the SBI boot protocol. Sparse — never an array index.
    hartid: AtomicUsize,
    /// Dense logical index, `0..cpus`: the array subscript.
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
///
/// Also what `device_tree` records hart ids up to, so a machine with more of them is
/// reported where they are found rather than after `memory::stack` has allocated and
/// mapped a stack for each.
pub const MAX_CPUS: usize = 64;

/// Slot 0 belongs to the firmware-selected boot hart.
static CPU_SLOTS: [CpuSlot; MAX_CPUS] = [const { CpuSlot::new() }; MAX_CPUS];
static BOOT_READY: AtomicBool = AtomicBool::new(false);

/// This hart's control block, or `None` before one was adopted.
///
/// The architecture entry zeroes `tp` so this question has an answer at all — firmware
/// leaves garbage there, indistinguishable from a live block. Only the console should
/// ask: it prints from inside the window before [`init_boot`], where panicking for want
/// of a hart id would lose the message worth having. Everything else uses [`current`].
fn try_current() -> Option<&'static Cpu> {
    let tp: usize;
    // SAFETY: reading a register.
    unsafe { core::arch::asm!("mv {}, tp", out(reg) tp, options(nomem, nostack)) };
    // SAFETY: `tp` is either zero, or a pointer `install_current` took from the
    // static CPU slot array.
    (tp != 0).then(|| unsafe { &*(tp as *const Cpu) })
}

/// This hart's control block.
///
/// # Panics
/// If `tp` is null, which means the Rust entry did not adopt a CPU first.
pub fn current() -> &'static Cpu {
    try_current().expect("tp is null: the boot entry did not adopt a Cpu")
}

/// This hart's physical id, or `None` before one was adopted; every `[hart N]` prefix
/// comes from here. Callers that know a block exists should say so with [`current`].
pub fn try_hart_id() -> Option<usize> { try_current().map(Cpu::hartid) }

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

/// Every hart the machine reports except the boot hart, capped at the cpu slots left.
///
/// Defined against [`boot_hart`], not against whoever is asking, so the answer does not
/// depend on which hart calls it. Iterated, never ranged over: ids need not be `0..n`.
///
/// Slot 0 is the boot hart's, so the rest of the machine gets `MAX_CPUS - 1`. Capped here
/// rather than at [`start_secondaries`], which runs once [`crate::memory::stack`] has
/// already allocated and mapped a stack for every hart this yields.
///
/// Consumed once, by [`crate::memory::init`], which stores the hart-to-stack pairing;
/// [`start_secondaries`] reads that back rather than walking this again.
///
/// # Panics
/// Before [`init_boot`], when "except the boot hart" has no meaning yet.
pub fn secondary_hart_ids() -> impl Iterator<Item = usize> {
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

/// Bring up every hart [`crate::memory::init`] reserved a stack for.
///
/// Call once, from the boot hart, **after** [`crate::memory::init`]: each hart is handed a
/// stack that only the kernel page table maps, so both must exist first.
pub fn start_secondaries() {
    let entry = virt_to_phys(boot::secondary_entry_address());
    let satp = kernel_table::satp()
        .expect("no kernel page table published; start_secondaries ran before memory::init");

    // Slot 0 belongs to the boot hart, so secondaries start at 1.
    let mut requested = 0;
    for (index, &stack::Secondary { hart, stack }) in (1..).zip(stack::secondaries()) {
        assert!(index < MAX_CPUS, "cpu slot {index} is out of range; secondary_hart_ids caps it");
        let slot = &CPU_SLOTS[index];
        // Ask first: "already started" and "no such hart" are different problems, and
        // hart_start's error code does not distinguish them.
        match sbi::hart_get_status(hart) {
            Ok(sbi::HartState::Stopped) => {}
            Ok(state) => {
                println!("[smp] hart {hart} not started: firmware reports {state:?}");
                continue;
            }
            Err(error) => {
                println!("[smp] hart {hart} status unavailable: {error:?}");
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
        match sbi::hart_start(hart, entry, opaque) {
            Ok(()) => {
                requested += 1;
                println!(
                    "[smp] started hart {hart} (cpu {index}) at {entry:#x}, stack top {:#x}",
                    stack.top()
                )
            }
            Err(error) => println!("[smp] hart {hart} failed to start: {error:?}"),
        }
    }

    await_secondaries(requested);
}

/// Wait for the harts we asked for to arrive, and say so if they do not.
///
/// `hart_start` returning `Ok` means only that firmware accepted the request. Without
/// confirming arrival, a secondary that faults in the stackless entry parks forever while
/// `kmain` prints its success line — a boot with N-1 dead harts looks like a good one.
///
/// Bounded by a duration, not a spin count: a count measures the *host's* speed, and one
/// generous on real hardware can exceed a minute under TCG, making the timeout
/// indistinguishable from the hang it reports. Where a second comes from is
/// [`crate::time`]'s; without a clock the wait is skipped and said to be, since a hart's
/// arrival is logged individually either way.
fn await_secondaries(requested: usize) {
    /// Long enough that a slow emulated hart is not slandered, short enough that a
    /// genuinely dead one does not look like a hang.
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

    // Reported rather than fatal: losing a secondary leaves the boot hart able to run,
    // and a kernel that can still say what went missing beats one that stops.
    let online = ONLINE.load(Ordering::Acquire);
    println!(
        "[smp] WARNING: {} of {requested} secondaries never reached the kernel after \
         {TIMEOUT_SECS}s; inspect sepc/scause/stval",
        requested - online
    );
}

/// Report what this hart is. The image layout is [`crate::memory::layout::report`]'s.
pub fn print_info() {
    match boot_hart() {
        Some(hart) => println!("boot hart: {hart} (chosen by the firmware)"),
        None => println!("boot hart: unrecorded"),
    }
}
