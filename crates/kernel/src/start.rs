use crate::cpu;
use crate::memory;

// static mut KERNEL_STARTED: bool = false;

/// Boot-hart entry, called from `boot.S`. Entered in **S-mode** by the SBI firmware
/// (OpenSBI): `a0 = hartid`, `a1 = dtb`. All M-mode setup (PMP, trap delegation,
/// timer) was done by the SBI, so there is no `mret` here — we are already in
/// supervisor mode.
///
/// Exactly one hart reaches this — `boot.S` claims the single boot stack before
/// jumping here and parks anything that loses; [`cpu::record_boot_hart`] says which
/// hart that turns out to be and why it is not predictable. Secondary harts enter at
/// [`secondary_start`] instead, so there is no runtime branch on "am I the boot hart":
/// the two roles are two entry points.
///
/// `va_offset` is the VA↔PA skew `boot.S` measured from the linked-vs-real
/// address of its high-half jump. It is not used to translate — that is a
/// compile-time constant now — only to prove reality matches it.
#[unsafe(no_mangle)]
unsafe extern "C" fn start(hartid: usize, dtb: usize, va_offset: usize) -> ! {
    // Fail loudly and immediately if the linker script and Rust disagree about
    // where the direct map lives; every address below depends on it.
    memory::direct_map::verify(va_offset);

    // Tie the two carriers of this hart's id together before anything uses either.
    cpu::adopt(hartid);
    cpu::record_boot_hart(hartid);

    // Parse the DTB the SBI handed us in a1: it populates the device table
    // (the console learns the UART base from here). Zero-allocation, so it
    // is safe to run before the heap exists.
    unsafe { crate::device_tree::init(dtb) };
    crate::device_tree::summary();
    cpu::print_info();

    println!("initializing memory...");
    // `memory` owns the ordering of what it brings up; see `memory::init`. It is
    // handed the hart list rather than looking it up, because which harts this kernel
    // will start is `cpu`'s decision and `cpu` already depends on `memory` — asking
    // upwards would make that circular. This is the one place that knows both.
    memory::init(cpu::secondary_hart_ids());
    println!("initializing memory completed");

    // Only now: each secondary is handed a stack that lives above the direct map,
    // which nothing but the published kernel table describes.
    cpu::start_secondaries();

    // No trap init here — see the note on `secondary_start`. `stvec` still points at
    // `boot.S`'s `_trap_park`.

    // Already in S-mode — go straight into the kernel.
    unsafe { kmain() }
}

/// Secondary-hart entry, called from `boot.S`. `a0 = hartid`.
///
/// Nothing global to set up, and nothing to wait for. By the time a hart is here it
/// is already running on the kernel page table and on a stack the boot hart
/// allocated, mapped and passed to it — `boot.S` installs both before there is a
/// stack to run Rust on. That is what makes re-initialising the allocator over live
/// RAM impossible here rather than merely discouraged.
#[unsafe(no_mangle)]
unsafe extern "C" fn secondary_start(hartid: usize) -> ! {
    cpu::adopt(hartid);
    // Before the log line, not after: this is what `start_secondaries` waits on, and
    // the console is far slower than that wait needs to be.
    cpu::record_online();
    println!(
        "[smp] hart {hartid} (cpu {}) online on the kernel page table",
        cpu::current().index()
    );

    // No trap init on any hart, boot or secondary: the trap subsystem is parked in
    // `crates/kernel/attic/trap/` while the boot and memory-init path is finalised.
    //
    // `stvec` is not left undefined. `boot.S` points every hart at `_trap_park`
    // before Rust runs and re-points it at the high alias after the jump, so a trap
    // stops the faulting hart deterministically with `scause`/`sepc`/`stval` intact.
    // In this phase every trap is a bug, and a handler could do nothing about it
    // that parking does not.

    unsafe { kmain_ap() }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn kmain() -> ! {
    println!("enter kmain");

    println!("This is my operating system!");
    println!("[kmain] higher-half kernel is live at high VAs — parking.");

    // Nothing to run yet, and no timer to wake us: `sstatus.SIE` is clear and no
    // source is enabled, so this `wfi` loop is a true halt rather than an idle.
    // Reaching this line is the success condition for the boot and memory-init
    // phase — the kernel got here, on the kernel page table, at high VAs, without
    // faulting.
    crate::arch::riscv64::wait_forever()
}

#[unsafe(no_mangle)]
// mark the function as extern "C" to tell the compiler that it should use the C calling convention for this function
unsafe extern "C" fn kmain_ap() -> ! {
    println!("enter kmain_ap (running on the kernel page table)");

    // Park. There is no scheduler to enter, so `wfi` is what a hart with nothing to
    // run should do. With no interrupts enabled it never wakes, which is correct for
    // this phase; it becomes a real idle loop on its own once traps come back.
    crate::arch::riscv64::wait_forever()
}
