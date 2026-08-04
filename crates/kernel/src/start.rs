use crate::cpu;
use crate::memory;

/// First ordinary Rust code on the boot hart.
pub(crate) unsafe extern "C" fn boot(hartid: usize, dtb: usize, va_offset: usize) -> ! {
    cpu::init_boot(hartid);
    memory::direct_map::verify(va_offset);

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

    kmain_ap()
}

fn kmain() -> ! {
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

fn kmain_ap() -> ! {
    println!("enter kmain_ap (running on the kernel page table)");

    // Park. There is no scheduler to enter, so `wfi` is what a hart with nothing to
    // run should do. With no interrupts enabled it never wakes, which is correct for
    // this phase; it becomes a real idle loop on its own once traps come back.
    crate::arch::riscv64::wait_forever()
}
