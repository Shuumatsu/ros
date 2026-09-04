//! Boot and secondary-hart Rust entry points.
//!
//! Each hart installs its trap vector before enabling an interrupt source.

use mmu::PhysicalAddr;

use crate::cpu;
use crate::memory;
use crate::process;
use crate::time;
use crate::trap;

pub(crate) unsafe extern "C" fn boot(hartid: usize, dtb: usize) -> ! {
    // SAFETY: this is the first Rust operation, before static access or secondary startup.
    unsafe { memory::layout::clear_bss() };

    cpu::init_boot(hartid);

    // SAFETY: firmware supplied the DTB address, and initialization occurs once before secondaries.
    unsafe { crate::device_tree::init(PhysicalAddr::new(dtb)) };
    crate::device_tree::summary();
    cpu::print_info();

    println!("initializing memory...");
    let machine = crate::device_tree::machine_memory();
    memory::init_allocators(machine);
    cpu::assign_stacks();
    memory::init_page_table(machine);
    println!("initializing memory completed");

    // Install the vector before arming the first interrupt source.
    trap::init();
    time::timer::start();

    cpu::start_secondaries();

    kmain()
}

pub(crate) unsafe extern "C" fn secondary(hartid: usize, cpu_pointer: usize) -> ! {
    unsafe { cpu::init_secondary(hartid, cpu_pointer) };
    cpu::record_online();
    println!(
        "[smp] hart {hartid} (cpu {}) online on the kernel page table",
        cpu::current().index()
    );

    trap::init();
    time::timer::start();

    kmain_ap()
}

fn kmain() -> ! {
    println!("enter kmain");

    println!("This is my operating system!");

    let status = process::run();
    println!("[kmain] hello exited with status {status}");

    println!("[kmain] higher-half kernel is live at high VAs — idling on the timer.");

    idle()
}

fn kmain_ap() -> ! {
    println!("enter kmain_ap (running on the kernel page table)");

    idle()
}

fn idle() -> ! {
    loop {
        crate::arch::idle();
    }
}
