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

        match sbi::hart_start(hart, entry, stack.top()) {
            Ok(()) => {
                println!("[smp] started hart {hart} at {entry:#x}, stack top {:#x}", stack.top())
            }
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
    // Logged because it varies from boot to boot (see `record_boot_hart`). Having it
    // in every log is what makes a hart-dependent failure obvious instead of
    // mysterious — and it is how the parked-boot-hart bug was finally pinned down.
    match boot_hart() {
        Some(hart) => println!("boot hart: {hart} (chosen by the firmware, not assumed)"),
        None => println!("boot hart: unrecorded"),
    }
}
