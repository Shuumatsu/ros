//! Which harts exist, which one is running this code, and bringing the rest up.
//!
//! Hart identity lives here rather than in the ISA layer: `tp` points at this hart's
//! [`Cpu`], and reading a field out of it is this module's business.
//!
//! Two numbering schemes, deliberately distinct. A *hart id* is the platform's, sparse
//! and firmware-chosen; a *cpu index* is dense, `0..cpus`, assigned here, and the only
//! one that subscripts anything. Slot 0 is the boot hart.
//!
//! **Nothing indexes by hart id.** Ids are only promised to be unique, and real platforms
//! leave gaps, so an array indexed by id costs storage proportional to the largest id and
//! mis-slots the boot hart on a machine that picks a large one.
//!
//! [`start_secondaries`] tells a starting hart two things and lets it derive nothing:
//! the stackless entry to begin at, and one release-published handoff carrying the page
//! table, stack top and prepared [`Cpu`]. Everything it needs is therefore chosen by a
//! hart that already has a heap and a page table.
//!
//! Which hart runs on which stack is this module's, in the slot beside the hart's [`Cpu`].
//! [`crate::memory::stack`] owns what a stack *is* — its size, its guard page, where in the
//! address space it goes — and hands one over on request; pairing it with a hart is a fact
//! about this machine's processors, and a memory subsystem holding the roster would be
//! answering a question `cpu` is asked.

use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use spin::Once;

use crate::arch::{self, boot};
use crate::memory::kernel_table;
use crate::memory::stack::{self, Stack};
use crate::println;
use crate::time;

/// Per-hart control block, and where per-hart state lives. `tp` points at this hart's.
///
/// A pointer rather than a bare id, as Linux does: `tp` is the one register that is
/// per-hart for free, and spending it on an integer already handed to us in `a0` would
/// leave every future piece of per-hart state needing a home and a lookup. A subsystem with
/// something to keep per hart puts a field here and reaches it through [`current`]; the
/// alternative — an array of its own, subscripted by [`Cpu::index`] — is a second per-hart
/// storage scheme, one index lookup deeper and with every hart's copy packed into the same
/// cache line.
///
/// Atomics because the block is shared state as far as the compiler is concerned. They are
/// never contended: a `&Cpu` can only be obtained from [`current`], so a hart reaches nothing
/// but its own, and a plain load and store is enough where a counter shared between harts
/// would need a read-modify-write.
#[repr(C)]
pub struct Cpu {
    /// Physical hart id from the SBI boot protocol. Sparse — never an array index.
    hartid: AtomicUsize,
    /// Dense logical index, `0..cpus`: the array subscript.
    index: AtomicUsize,
    /// Timer interrupts this hart has taken, from [`crate::time::timer`].
    ticks: AtomicU64,
}

impl Cpu {
    const fn new() -> Self {
        Self {
            hartid: AtomicUsize::new(0),
            index: AtomicUsize::new(0),
            ticks: AtomicU64::new(0),
        }
    }

    /// Physical hart id, for SBI calls and diagnostics.
    pub fn hartid(&self) -> usize { self.hartid.load(Ordering::Relaxed) }

    /// Dense logical index, for anything array-shaped.
    pub fn index(&self) -> usize { self.index.load(Ordering::Relaxed) }

    /// Count a timer tick on this hart, and answer with the new total.
    pub fn record_tick(&self) -> u64 {
        let ticks = self.ticks.load(Ordering::Relaxed) + 1;
        self.ticks.store(ticks, Ordering::Relaxed);
        ticks
    }
}

/// One hart's slot: its control block, the handoff that started it, and the stack it runs on.
///
/// A cache line to itself, so that a write to one hart's state does not evict another's. That
/// is what lets [`Cpu`] be the home for hot per-hart state — the separation is paid for once
/// here rather than by every subsystem that adds a field.
#[repr(C, align(64))]
struct CpuSlot {
    cpu: Cpu,
    handoff: boot::SecondaryHandoff,
    /// The stack this hart will run on, from [`assign_stacks`]. `Once`, because a hart
    /// handed a second stack would keep pushing onto the first.
    ///
    /// Slot 0's stays empty: the boot hart is already running on the linker's stack, and
    /// nothing here has to tell it where.
    stack: Once<Stack>,
}

impl CpuSlot {
    const fn new() -> Self {
        Self { cpu: Cpu::new(), handoff: boot::SecondaryHandoff::new(), stack: Once::new() }
    }
}

/// Upper bound on harts this kernel will run.
///
/// Also what `device_tree` records hart ids up to, so a machine with more of them is
/// reported where they are found rather than after a stack has been allocated and mapped
/// for each — and what `memory::stack` sizes its list of stacks from, since one hart is
/// what a kernel stack is for.
pub const MAX_CPUS: usize = 64;

const _: () = assert!(
    align_of::<CpuSlot>() == arch::CACHE_LINE_BYTES,
    "a cpu slot must be aligned to a whole cache line, or two harts share one"
);

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
    let tp = arch::thread_pointer();
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
    // SAFETY: a `&'static Cpu` out of `CPU_SLOTS`, which is what every reader of `tp`
    // expects to find there.
    unsafe { arch::set_thread_pointer(cpu as *const Cpu as usize) };
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
/// rather than at [`start_secondaries`], which runs once a stack has been allocated and
/// mapped for every hart this yields.
///
/// Walked once, by [`assign_stacks`], which fills a slot per hart; [`start_secondaries`]
/// reads the slots back rather than walking this again.
///
/// # Panics
/// Before [`init_boot`], when "except the boot hart" has no meaning yet.
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

/// How many slots [`assign_stacks`] filled, so `CPU_SLOTS[1..=this]` are the secondaries.
///
/// A `Once`, so [`start_secondaries`] can tell "no secondaries" from "nobody has assigned
/// them yet" — the second would otherwise start no harts and say nothing.
static SECONDARIES: Once<usize> = Once::new();

/// Claim a cpu slot and a kernel stack for every hart this kernel means to start.
///
/// Call once, from the boot hart, between [`crate::memory::init_allocators`] and
/// [`crate::memory::init_page_table`]: a stack needs frames and an address, and the table
/// built by the second has to map it before its hart ever pushes.
///
/// Only the slot's *contents* are decided here. Whether firmware will actually start the
/// hart is [`start_secondaries`]' question, asked later and separately, because a hart that
/// refuses to start still had a stack reserved for it and saying so needs the pairing to
/// exist.
///
/// # Panics
///
/// If called twice. The second call would hand already-assigned harts nothing — every slot
/// is a `Once` — while [`SECONDARIES`] kept the first count.
pub fn assign_stacks() {
    assert!(SECONDARIES.get().is_none(), "cpu::assign_stacks called twice; the slots are filled");

    // Slot 0 belongs to the boot hart, so secondaries start at 1.
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
/// Call once, from the boot hart, **after** [`crate::memory::init_page_table`]: each hart is
/// handed a stack that only the kernel page table maps, so both must exist first.
pub fn start_secondaries() {
    let satp = kernel_table::satp()
        .expect("no kernel page table published; start_secondaries ran before memory");
    let secondaries = *SECONDARIES
        .get()
        .expect("no cpu slots assigned; start_secondaries ran before cpu::assign_stacks");
    let entry = boot::entry_address();

    // The slots `assign_stacks` filled, which already know their own hart and index —
    // recomputing either here would be a second answer to what a slot is. What it takes to
    // actually start one is `boot::start_cpu`'s; this decides who, with what, and says so.
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
            // A hart the firmware keeps for itself is a fact about the machine; a refused
            // start or an unreadable status is a fault, and the two must not read alike in
            // a log where the second one is what you are looking for.
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
