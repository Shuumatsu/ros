use crate::cpu;
use crate::memory;
use crate::trap;

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

    println!("initializing traps...");
    // Per-hart: `stvec` is a CSR, so every hart sets its own.
    unsafe { trap::init() };
    println!("initializing traps completed");

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
    println!("[smp] hart {hartid} online on the kernel page table");

    // Per-hart: `stvec` is a CSR, so every hart sets its own.
    unsafe { trap::init() };

    unsafe { kmain_ap() }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn kmain() -> ! {
    println!("enter kmain");

    println!("This is my operating system!");
    println!("[kmain] higher-half kernel is live at high VAs — parking.");

    // Nothing to run yet. The next milestone is user-process support (per-process
    // page tables + U=1 user pages + a real syscall path), which will replace the
    // old in-place ecall demo. Park until then; the timer keeps ticking.
    crate::arch::riscv64::wait_forever()
}

#[unsafe(no_mangle)]
// mark the function as extern "C" to tell the compiler that it should use the C calling convention for this function
unsafe extern "C" fn kmain_ap() -> ! {
    println!("enter kmain_ap (running on the kernel page table)");

    // Park. There is no scheduler to enter, and the previous placeholder here was a
    // tight `ebreak` loop — harmless only because no secondary hart had ever actually
    // reached it. The moment one did it would trap on every iteration and bury the
    // console. `wfi` idles until an interrupt instead, which is what a hart with
    // nothing to run should do.
    crate::arch::riscv64::wait_forever()
}
