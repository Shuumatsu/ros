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

    if hartid == 0 {
        // Parse the DTB the SBI handed us in a1: it populates the device table
        // (the console learns the UART base from here). Zero-allocation, so it
        // is safe to run before the heap exists.
        unsafe { crate::device_tree::init(dtb) };
        crate::device_tree::summary();
        cpu::print_info();
    }

    println!("initializing memory...");
    memory::init();
    // Replace boot.S's blanket-RWX gigapage table with a real one: per-section
    // rights and W^X. Must follow memory::init — it allocates frames.
    memory::kernel_table::init();
    println!("initializing memory completed");

    println!("initializing traps...");
    unsafe { trap::init() };
    println!("initializing traps completed");

    // Already in S-mode — go straight into the kernel.
    if hartid == 0 {
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
