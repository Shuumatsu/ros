use core::sync::atomic::{AtomicUsize, Ordering};

use paging::MemoryAddr;

use crate::arch::riscv64::sbi;
use crate::memory::{kernel_table, layout, stack, virt_to_phys};
use crate::println;

/// Per-hart control block. `tp` points at this hart's.
///
/// Following Linux, which keeps a pointer here rather than a bare id: `tp` is the
/// one register that is per-hart for free, so spending it on an integer we were
/// already handed in `a0` means every future piece of per-hart state needs a new
/// home and a new lookup.
///
/// # Layout is load-bearing
///
/// `boot.S` reads and writes this in assembly before there is a stack to run Rust
/// on, by fixed offset. The `offset_of!` assertions below are what keep the two in
/// step; reorder a field and the build fails rather than the boot.
///
/// Fields are atomics because `boot.S` writes `hartid` from assembly while Rust
/// holds a shared reference to the same block.
#[repr(C)]
pub struct Cpu {
    /// Top of this hart's stack. **Offset 0**: `boot.S` loads `sp` from here, which
    /// is why the whole block can travel in SBI's single `opaque` word.
    stack_top: AtomicUsize,
    /// Physical hart id from the SBI boot protocol. Sparse — never an array index.
    hartid: AtomicUsize,
    /// Dense logical index, `0..cpus`. This is the array subscript; `hartid` is not.
    index: AtomicUsize,
}

const _: () = assert!(core::mem::offset_of!(Cpu, stack_top) == 0, "boot.S loads sp from 0(tp)");
const _: () = assert!(core::mem::offset_of!(Cpu, hartid) == 8, "boot.S stores a0 to 8(tp)");

impl Cpu {
    const fn new() -> Self {
        Self {
            stack_top: AtomicUsize::new(0),
            hartid: AtomicUsize::new(0),
            index: AtomicUsize::new(0),
        }
    }

    /// Physical hart id, for SBI calls and diagnostics.
    pub fn hartid(&self) -> usize {
        self.hartid.load(Ordering::Relaxed)
    }

    /// Dense logical index, for anything array-shaped.
    pub fn index(&self) -> usize {
        self.index.load(Ordering::Relaxed)
    }
}

/// Upper bound on harts this kernel will run. Matches what the device tree module
/// will report; the blocks are `.bss`, so the whole array costs 1.5 KiB.
const MAX_CPUS: usize = 64;

/// Every hart's control block. Slot 0 is the boot hart's — `boot.S` points its `tp`
/// straight at this symbol, so slot 0 must stay first.
#[unsafe(no_mangle)]
static KERNEL_CPUS: [Cpu; MAX_CPUS] = [const { Cpu::new() }; MAX_CPUS];

/// This hart's control block.
///
/// # Panics
/// If `tp` is null, which means `boot.S` did not point it at a block before
/// entering Rust.
pub fn current() -> &'static Cpu {
    let tp: usize;
    // SAFETY: reading a register.
    unsafe { core::arch::asm!("mv {}, tp", out(reg) tp, options(nomem, nostack)) };
    assert!(tp != 0, "tp is null: boot.S must point it at a Cpu block before calling Rust");
    // SAFETY: `boot.S` sets `tp` from `KERNEL_CPUS`, either slot 0 directly or a slot
    // address the boot hart passed through SBI's `opaque`. Both are `'static`.
    unsafe { &*(tp as *const Cpu) }
}

/// This hart's physical id. Every `[hart N]` console prefix comes from here.
pub fn hart_id() -> usize {
    current().hartid()
}

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

/// Reconcile the two carriers of "which hart am I".
///
/// Every hart calls this once, first thing, boot or secondary.
///
/// The SBI boot protocol hands the id in `a0`, which arrives as the `hartid`
/// argument. `boot.S` also stores it into this hart's [`Cpu`] block, which is what
/// [`hart_id`] reads and therefore where every `[hart N]` console prefix comes from.
/// One value, two carriers; this is what stops them drifting apart silently.
pub fn adopt(hartid: usize) {
    let from_block = current().hartid();
    assert_eq!(
        hartid, from_block,
        "hart id disagreement: the SBI boot protocol says {hartid}, this hart's Cpu \
         block says {from_block}. boot.S must store a0 into 8(tp) on both entry paths"
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
/// - `opaque`, which lands in the hart's `a1`, is the address of its [`Cpu`] block,
///   already filled in with the stack top and logical index. One word carries both
///   because `stack_top` is the block's first field, so `boot.S` can load `sp` from
///   `0(tp)`. The hart computes no address of its own.
pub fn start_secondaries() {
    let entry = virt_to_phys(layout::secondary_entry());
    // Not a wait: the value only exists because the table is live, and it is what
    // `boot.S` reads to get onto it. Checking it here turns a would-be silent hang
    // on the far side into a panic on this one.
    assert!(
        kernel_table::satp().is_some(),
        "no kernel page table published; start_secondaries ran before memory::init"
    );

    // Slot 0 belongs to the boot hart, so secondaries start at 1.
    let mut requested = 0;
    for (slot, &stack::Secondary { hart, stack }) in
        (1..).zip(stack::secondaries())
    {
        assert!(
            slot < MAX_CPUS,
            "machine reports more than {MAX_CPUS} harts; raise MAX_CPUS"
        );
        let block = &KERNEL_CPUS[slot];
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
        assert!(
            stack.top().is_aligned(16),
            "hart {hart}'s stack top {:#x} is not 16-byte aligned; boot.S loads it \
             directly into sp",
            stack.top()
        );

        // Fill the block before the hart can reach it. `boot.S` loads `sp` from
        // `stack_top` as its first act, so this store has to be visible first;
        // `hart_start` is an `ecall` through the firmware, which orders it.
        block.stack_top.store(stack.top().bits(), Ordering::Relaxed);
        block.index.store(slot, Ordering::Relaxed);
        block.hartid.store(hart, Ordering::Release);

        let opaque = block as *const Cpu as usize;
        match sbi::hart_start(hart, entry.bits(), opaque) {
            Ok(()) => {
                requested += 1;
                println!(
                    "[smp] started hart {hart} (cpu {slot}) at {entry:#x}, stack top {:#x}",
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
/// inside `boot.S` (a bad stack mapping, say) parks in `_trap_park` forever while
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
