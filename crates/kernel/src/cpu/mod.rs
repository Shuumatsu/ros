use core::sync::atomic::{AtomicUsize, Ordering};

use crate::arch::riscv64::sbi;
use crate::memory::{kernel_table, layout, stack, virt_to_phys};
use crate::println;

/// Sentinel for "no hart has recorded the boot role yet".
///
/// A boot hart whose id really were `usize::MAX` would be reported as unrecorded.
/// That costs one wrong word in one log line and nothing else — [`boot_hart`] is
/// diagnostic, and no hart id is used as an index anywhere. It is a sentinel, not a
/// claim about the id space.
const UNCLAIMED: usize = usize::MAX;

/// Which hart ran the one-time kernel initialisation.
static BOOT_HART: AtomicUsize = AtomicUsize::new(UNCLAIMED);

/// Secondary harts that have reached [`crate::start::secondary_start`].
static ONLINE: AtomicUsize = AtomicUsize::new(0);

/// Reconcile the two answers to "which hart am I", and adopt the id.
///
/// Every hart calls this once, first thing, boot or secondary.
///
/// # Why there are two answers to reconcile
///
/// The SBI boot protocol hands the id in `a0`, which arrives as the `hartid`
/// argument to the entry points. Separately, `boot.S` does `mv tp, a0` and
/// [`crate::arch::riscv64::hart_id`] reads `tp` back — that is where every
/// `[hart N]` console prefix comes from. Two independent carriers of one fact, and
/// until now nothing compared them.
///
/// It is visible in a single line of output: `secondary_start` logs
/// `[smp] hart {hartid} online`, and the console prefixes it with `[hart {tp}]` —
/// the same number, printed twice, from two sources that could disagree.
///
/// The disagreement is not exotic. `tp`'s natural next use is a pointer to a
/// per-hart control block, which is what Linux keeps there and what a scheduler
/// will want; the id then becomes a field in that block rather than the register's
/// whole contents. On the day `boot.S`'s `mv tp, a0` is repurposed, every log line
/// and every future `hart_id()` caller silently reports garbage while `BOOT_HART`
/// keeps reporting the truth. Nothing would fail, and nothing would assert.
///
/// So `cpu` owns hart identity and the `tp` convention answers to it. Checking the
/// two agree costs one comparison per hart per boot and turns that future silent
/// divergence into an immediate panic.
pub fn adopt(hartid: usize) {
    let from_tp = crate::arch::riscv64::hart_id();
    assert_eq!(
        hartid, from_tp,
        "hart id disagreement: the SBI boot protocol says {hartid}, tp says {from_tp}. \
         boot.S must keep `mv tp, a0` in step with what it passes to the entry point"
    );
}

/// Record that a secondary hart made it into Rust, on the kernel page table.
pub fn record_online() {
    ONLINE.fetch_add(1, Ordering::Release);
}

/// Record which hart ran the one-time initialisation.
///
/// # This is not the election
///
/// `boot.S` elects the boot hart, with an `lr`/`sc` claim taken before anything
/// else runs: there is exactly one boot stack, so exactly one hart may proceed and
/// the rest park. Electing again here would be a second answer to the same
/// question. This records the winner — and asserts the property, so a second
/// arrival is a loud panic rather than two harts quietly sharing a stack.
///
/// # Why the winner is not hart 0
///
/// **The owner of this fact; elsewhere just points here.** The previous boot stage
/// picks which hart enters the kernel and is *not required to pick 0* — OpenSBI's
/// boot hart is configurable, and on QEMU virt it is a lottery that genuinely varies
/// from boot to boot: at `-smp 32` it has been observed as 0, 5, 8, 13, 21, 24, 25
/// and 31 across consecutive runs of the same image.
///
/// So nothing may assume a value here, and nothing may assume a *range* either —
/// which is the same mistake one step removed, and the one that used to park the boot
/// hart outright. See [`crate::memory::stack`].
pub fn record_boot_hart(hartid: usize) {
    BOOT_HART
        .compare_exchange(UNCLAIMED, hartid, Ordering::AcqRel, Ordering::Acquire)
        .unwrap_or_else(|winner| {
            panic!(
                "hart {hartid} reached the boot path, but hart {winner} already holds the \
                 boot stack; boot.S's claim did not hold"
            )
        });
}

/// The hart that ran the one-time initialisation, once one has recorded it.
pub fn boot_hart() -> Option<usize> {
    match BOOT_HART.load(Ordering::Acquire) {
        UNCLAIMED => None,
        hart => Some(hart),
    }
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
/// Before [`record_boot_hart`], since "every hart except the boot hart" has no
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
/// - `start_addr` is the *physical* address of `_secondary_start`. SBI starts a hart
///   with `satp = 0` — translation off — so this cannot be a Rust function at a high
///   virtual address, and it is not `_start` either: that entry is the boot hart's,
///   with the image header and the one-time BSS zeroing behind it.
/// - `opaque`, which lands in the hart's `a1`, is the top of the stack allocated for
///   it. The hart computes no address of its own; see [`crate::memory::stack`] for
///   why deriving one from a hart id was the bug this replaced.
pub fn start_secondaries() {
    let entry = virt_to_phys(layout::secondary_entry());
    // Not a wait: the value only exists because the table is live, and it is what
    // `boot.S` reads to get onto it. Checking it here turns a would-be silent hang
    // on the far side into a panic on this one.
    assert!(
        kernel_table::satp().is_some(),
        "no kernel page table published; start_secondaries ran before memory::init"
    );

    let mut requested = 0;
    for &stack::Secondary { hart, stack } in stack::secondaries() {
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

        // The stack top is what the hart will load straight into `sp`, and the
        // RISC-V ABI requires sp to be 16-byte aligned. `boot.S` does `mv sp, a1`
        // with no arithmetic and no check, so this is the only place the property
        // can be enforced; it holds today only because SIZE and GUARD_SIZE happen
        // to be page multiples.
        assert_eq!(
            stack.top() % 16,
            0,
            "hart {hart}'s stack top {:#x} is not 16-byte aligned; boot.S loads it \
             directly into sp",
            stack.top()
        );

        match sbi::hart_start(hart, entry, stack.top()) {
            Ok(()) => {
                requested += 1;
                println!("[smp] started hart {hart} at {entry:#x}, stack top {:#x}", stack.top())
            }
            Err(error) => println!("[smp] hart {hart} failed to start: {error}"),
        }
    }

    await_secondaries(requested);
}

/// Wait for the harts we asked for to actually arrive, and say so if they do not.
///
/// `hart_start` returning `Ok` means only that the firmware *accepted* the request —
/// the hart is `StartPending`. Nothing used to check any further, so a secondary that
/// faulted inside `boot.S` (a bad stack mapping, say) parked in `.Ltrap_park` forever
/// while the boot hart sailed on and `kmain` printed its "success condition" line. A
/// boot with N-1 dead harts looked exactly like a good one.
///
/// # Why the bound is a duration and not a spin count
///
/// The first version of this counted iterations, and that was wrong in a way worth
/// recording: a spin count measures the *host's* speed, not the guest's. 100 million
/// iterations was picked to be generously large and turned out to exceed 90 seconds
/// under QEMU's TCG interpreter — a "timeout" indistinguishable from the hang it
/// exists to report. Tuning the number would have made it wrong on the next machine.
///
/// `rdtime` reads the `time` CSR, which is readable in S-mode because OpenSBI sets
/// `[m|s]counteren.TM`, and `/cpus/timebase-frequency` says what its ticks are worth.
/// That is a real second on any host. Note this needs no timer *interrupt* and so
/// does not depend on the parked trap subsystem — reading the counter is just a CSR
/// read.
///
/// If the tree omitted the frequency there is no clock to bound by, so the wait is
/// skipped entirely and said so. Spinning on an unknown timebase would be guessing.
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
         {TIMEOUT_SECS}s; they are parked in boot.S with sepc/scause/stval intact",
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
    // Logged because it varies from boot to boot (see `record_boot_hart`). Having it
    // in every log is what makes a hart-dependent failure obvious instead of
    // mysterious — and it is how the parked-boot-hart bug was finally pinned down.
    match boot_hart() {
        Some(hart) => println!("boot hart: {hart} (chosen by the firmware, not assumed)"),
        None => println!("boot hart: unrecorded"),
    }
}
