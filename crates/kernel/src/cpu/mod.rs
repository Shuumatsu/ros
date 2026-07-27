use core::sync::atomic::{AtomicUsize, Ordering};

use crate::arch::riscv64::sbi;
use crate::memory::stack;
use crate::{print, println};

/// Sentinel for "no hart has claimed the boot role yet". A real hartid can never
/// reach it — `boot.S` parks anything at or above the hart count.
const UNCLAIMED: usize = usize::MAX;

/// Which hart ran the one-time kernel initialisation.
static BOOT_HART: AtomicUsize = AtomicUsize::new(UNCLAIMED);

/// Claim the boot-hart role, returning `true` for exactly one caller.
///
/// # Why this is not `hart_id() == 0`
///
/// The previous boot stage picks which hart enters the kernel, and it is *not
/// required to be hart 0* — OpenSBI's boot hart is configurable and differs across
/// platforms. Gating one-time setup on `hartid == 0` therefore risks a boot where
/// nothing runs `device_tree::init` and every later step fails for an unrelated
/// reason. Claiming the role instead makes it whoever actually arrived first, which
/// is the property that matters.
///
/// (On QEMU virt with OpenSBI this happens to be hart 0, so the distinction is
/// latent rather than currently observable.)
pub fn claim_boot_hart(hartid: usize) -> bool {
    BOOT_HART
        .compare_exchange(UNCLAIMED, hartid, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

/// The hart that ran the one-time initialisation, once one has claimed it.
pub fn boot_hart() -> Option<usize> {
    match BOOT_HART.load(Ordering::Acquire) {
        UNCLAIMED => None,
        hart => Some(hart),
    }
}

/// Bring up every other hart the machine reports.
///
/// Call once, from the boot hart, **after** [`crate::memory::init`]: a secondary
/// spins waiting for the kernel page table to be published, so starting one earlier
/// only makes it wait longer.
///
/// # Why each hart re-runs `boot.S`
///
/// SBI starts a hart with `satp = 0` — translation off — so the entry point must be
/// the *physical* address of `_start`, not a Rust function at a high virtual address.
/// Each secondary therefore installs the early table and jumps high exactly as the
/// boot hart did, then diverges at [`claim_boot_hart`], which it loses.
///
/// # Harts we refuse to start
///
/// A hart with no reserved stack would compute an `sp` inside its neighbour's stack.
/// `boot.S` parks such a hart on arrival, but it is better not to invite it: the
/// machine's hart count and the kernel's stack count are separate facts (see
/// [`crate::device_tree::hart_ids`]) and this is where they meet.
pub fn start_secondaries() {
    let me = crate::arch::riscv64::hart_id();
    let entry = crate::memory::virt_to_phys(crate::memory::layout::text_start());
    let servable = stack::max_harts();

    for &hart in crate::device_tree::hart_ids() {
        if hart == me {
            continue;
        }
        if hart >= servable {
            println!("[smp] hart {hart} not started: only {servable} harts have stacks");
            continue;
        }

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

        // `opaque` lands in the hart's `a1`, where `boot.S` expects the device tree
        // pointer. A secondary never parses it — that is boot-hart work — so the
        // value is irrelevant and passed as zero rather than pretending otherwise.
        match sbi::hart_start(hart, entry, 0) {
            Ok(()) => println!("[smp] started hart {hart} at {entry:#x}"),
            Err(error) => println!("[smp] hart {hart} failed to start: {error}"),
        }
    }
}

/// Report what this hart is, as opposed to what memory looks like.
///
/// The kernel image layout used to be printed here too, which put a function that
/// imports nothing but `memory::layout` and `memory::stack` in the CPU module. It
/// lives in [`crate::memory::report_layout`] now; this reports CPU facts only.
pub fn print_info() {
    // Logged because it is not a constant and not necessarily 0: OpenSBI runs a
    // lottery, so at `-smp 4` this varies from boot to boot. Having it in the log is
    // what makes a hart-dependent failure obvious instead of mysterious.
    match boot_hart() {
        Some(hart) => println!("boot hart: {hart} (chosen by the firmware, not assumed)"),
        None => println!("boot hart: unclaimed"),
    }
}
