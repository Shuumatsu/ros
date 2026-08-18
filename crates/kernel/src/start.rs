//! The two Rust entry points, and the bring-up order across subsystems.
//!
//! One function per kind of hart, because they arrive owing different things: the boot
//! hart owes `.bss`, the device tree, memory and the other harts; a secondary arrives on
//! a finished page table and owes only its own identity.
//!
//! The order in [`boot`] is this module's single contribution. No subsystem below knows
//! it, and each is handed what it needs rather than looking it up, which is what keeps
//! `device_tree`, `cpu` and `memory` free of dependencies on each other.
//!
//! Traps are the one part both entries owe equally, and in the same order: a vector before an
//! interrupt source, since a source armed on a hart still carrying the boot stage's park
//! vector is a hart that stops dead on its first tick.

use crate::cpu;
use crate::memory;
use crate::time;
use crate::trap;

/// First ordinary Rust code on the boot hart.
pub(crate) unsafe extern "C" fn boot(hartid: usize, dtb: usize) -> ! {
    // NOTHING may go above this line. `.bss` has no bytes in the image, so until
    // this returns every static holds whatever was in that RAM beforehand, and
    // `init_boot` on the next line writes to one.
    unsafe { memory::layout::clear_bss() };

    cpu::init_boot(hartid);

    // The DTB from a1 fills the device table, which is where the console learns its UART
    // base. Zero-allocation, so it is safe before the heap exists.
    unsafe { crate::device_tree::init(dtb) };
    crate::device_tree::summary();
    cpu::print_info();

    println!("initializing memory...");
    // `memory` owns the ordering within each half; this owns the one step between them.
    // The machine description is handed in rather than looked up, since `device_tree` owns
    // it and already depends on `memory`.
    //
    // `cpu` goes in the middle because a hart's stack needs frames and an address from the
    // first half and has to be mapped by the second — anything else wanting an address of
    // its own belongs here too. Neither subsystem calls the other; this knows both.
    let machine = crate::device_tree::machine_memory();
    memory::init_allocators(machine);
    cpu::assign_stacks();
    memory::init_page_table(machine);
    println!("initializing memory completed");

    // A vector first, then the sources — and both before the secondaries, so that the wait
    // for them is the first thing the tick runs underneath.
    trap::init();
    time::timer::start();

    cpu::start_secondaries();

    kmain()
}

/// First ordinary Rust code on a secondary hart.
pub(crate) unsafe extern "C" fn secondary(hartid: usize, cpu_pointer: usize) -> ! {
    unsafe { cpu::init_secondary(hartid, cpu_pointer) };
    cpu::record_online();
    println!(
        "[smp] hart {hartid} (cpu {}) online on the kernel page table",
        cpu::current().index()
    );

    // Per hart, both of them: `stvec` and `sie` are CSRs, and the deadline is this hart's.
    trap::init();
    time::timer::start();

    kmain_ap()
}

fn kmain() -> ! {
    println!("enter kmain");

    println!("This is my operating system!");
    println!("[kmain] higher-half kernel is live at high VAs — idling on the timer.");

    idle()
}

fn kmain_ap() -> ! {
    println!("enter kmain_ap (running on the kernel page table)");

    idle()
}

/// What a hart does with no work: sleep until an interrupt, handle it, sleep again.
///
/// The loop is what makes it a wait rather than a halt — [`crate::arch::idle`] returns on
/// every interrupt taken. It becomes the scheduler's idle task once there is something else
/// to run; until then a tick is the only thing that wakes a hart, and reaching here on every
/// hart is the success condition for this phase.
fn idle() -> ! {
    loop {
        crate::arch::idle();
    }
}
