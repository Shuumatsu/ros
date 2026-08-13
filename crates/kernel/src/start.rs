use crate::cpu;
use crate::memory;

/// First ordinary Rust code on the boot hart.
pub(crate) unsafe extern "C" fn boot(hartid: usize, dtb: usize, va_offset: usize) -> ! {
    // NOTHING may go above this line. `.bss` has no bytes in the image, so until
    // this returns every static holds whatever was in that RAM beforehand, and
    // `init_boot` on the next line writes to one.
    unsafe { memory::layout::clear_bss() };

    cpu::init_boot(hartid);
    memory::direct_map::verify(va_offset);

    // The DTB from a1 fills the device table, which is where the console learns its UART
    // base. Zero-allocation, so it is safe before the heap exists.
    unsafe { crate::device_tree::init(dtb) };
    crate::device_tree::summary();
    cpu::print_info();

    println!("initializing memory...");
    // `memory` owns the ordering; it is handed the hart list rather than looking it up,
    // since `cpu` decides that and already depends on `memory`. This knows both.
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

    // A true halt, not an idle: `sstatus.SIE` is clear and no source is enabled, so
    // nothing will wake this. Reaching here is the success condition for this phase.
    crate::arch::riscv64::wait_forever()
}

fn kmain_ap() -> ! {
    println!("enter kmain_ap (running on the kernel page table)");

    // No scheduler to enter yet, so park. This becomes a real idle loop on its own once
    // traps come back.
    crate::arch::riscv64::wait_forever()
}
