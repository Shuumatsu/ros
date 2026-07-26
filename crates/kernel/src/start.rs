use core::arch::asm;

use crate::cpu;
use crate::memory;
use crate::trap;

// static mut KERNEL_STARTED: bool = false;

/// Kernel entry, called from `boot.S`. Entered in **S-mode** by the SBI firmware
/// (OpenSBI): `a0 = hartid`, `a1 = dtb`. All M-mode setup (PMP, trap delegation,
/// timer) was done by the SBI, so there is no `mret` here — we are already in
/// supervisor mode.
///
/// `va_offset` is the VA↔PA skew `boot.S` measured from the linked-vs-real
/// address of its high-half jump. It is not used to translate — that is a
/// compile-time constant now — only to prove reality matches it.
#[unsafe(no_mangle)]
unsafe extern "C" fn start(hartid: usize, dtb: usize, va_offset: usize) -> ! {
    // Fail loudly and immediately if the linker script and Rust disagree about
    // where the direct map lives; every address below depends on it.
    memory::direct_map::verify(va_offset);

    // One-time setup belongs to whichever hart arrived first, *not* to hart 0. The
    // previous boot stage chooses which hart enters the kernel and is not required
    // to choose 0; gating on `hartid == 0` would mean a platform whose boot hart is
    // 1 never parses the device tree and then fails somewhere unrelated.
    let boot_hart = cpu::claim_boot_hart(hartid);

    if boot_hart {
        // Parse the DTB the SBI handed us in a1: it populates the device table
        // (the console learns the UART base from here). Zero-allocation, so it
        // is safe to run before the heap exists.
        unsafe { crate::device_tree::init(dtb) };
        crate::device_tree::summary();
        cpu::print_info();

        println!("initializing memory...");
        // Frames, then the heap carved from them, then the real kernel page table.
        // `memory` owns that ordering; see `memory::init`.
        memory::init();
        println!("initializing memory completed");
    } else {
        // Physical memory and the heap are global and already up (or on their way);
        // this hart only needs to stop running on the boot table. Blocks until the
        // boot hart publishes.
        memory::init_secondary();
    }

    println!("initializing traps...");
    // Per-hart: `stvec` is a CSR, so every hart sets its own.
    unsafe { trap::init() };
    println!("initializing traps completed");

    // Already in S-mode — go straight into the kernel.
    if boot_hart {
        unsafe { kmain() }
    } else {
        unsafe { kmain_ap() }
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn kmain() -> ! {
    kprintln!("enter kmain");

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
    println!("enter kmain_ap");

    // while !KERNEL_STARTED {}

    // println!("initializing paging...");
    // memory::paging::init();
    // println!("initializing paging completed");

    scheduler();
}

fn scheduler() -> ! {
    loop {
        unsafe {
            asm!("ebreak", options(nomem, nostack));
        }
    }
}
