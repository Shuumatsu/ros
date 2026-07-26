use core::sync::atomic::{AtomicUsize, Ordering};

use crate::memory::layout::*;
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

pub fn print_info() {
    // Logged because it is not a constant and not necessarily 0: OpenSBI runs a
    // lottery, so at `-smp 4` this varies from boot to boot. Having it in the log is
    // what makes a hart-dependent failure obvious instead of mysterious.
    if let Some(hart) = boot_hart() {
        println!("boot hart: {hart} (chosen by the firmware, not assumed)");
    }
    println!("kernel image layout: ");
    println!("    load base: {:#x}", memory_start());
    println!("    text_start: {:#x}, text_end: {:#x}", text_start(), text_end());
    println!("    rodata_start: {:#x}, rodata_end: {:#x}", rodata_start(), rodata_end());
    println!("    data_start: {:#x}, data_end: {:#x}", data_start(), data_end());
    println!("    bss_start: {:#x}, bss_end: {:#x}", bss_start(), bss_end());
    println!(
        "    kernel_stack_start: {:#x}, kernel_stack_end: {:#x}",
        kernel_stack_start(),
        kernel_stack_end()
    );
    println!(
        "    hart stacks: {} x {} KiB, each above a {} KiB guard page",
        stack::max_harts(),
        stack::SIZE / 1024,
        stack::GUARD_SIZE / 1024
    );
    // Heap end is discovered from the device tree at runtime; see `memory::init`.
    println!("    heap_start: {:#x}", heap_start());
}
