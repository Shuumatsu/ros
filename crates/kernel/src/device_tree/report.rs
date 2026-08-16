//! Printing the resolved device map.
//!
//! Reads only the stored table — no re-parse — so printing is fully decoupled from
//! discovery. Kept in its own file because it is the one part of this module that runs
//! after the walk rather than as part of it, on the UART the walk itself found.

use super::table;
use crate::utils::ByteSize;

/// Print the resolved device map. Call after [`super::init`], which is what backs the
/// console with the real UART.
pub fn summary() {
    let Some(table) = table::get() else {
        println!("[dtb] not parsed");
        return;
    };

    // Size included so the frame reservation in `memory::frame` can be checked
    // against it straight from the boot log.
    println!("[dtb] blob at {:#x} (size {:#x})", table.blob.base, table.blob.size);
    println!(
        "[dtb] ram:   {:#x}..{:#x} ({})",
        table.ram.base,
        table.ram.end(),
        ByteSize(table.ram.size)
    );

    let uart = &table.uart;
    match uart.irq {
        Some(irq) => println!("[dtb] uart:  {:#x} (size {:#x}, irq {irq})", uart.base, uart.size),
        None => println!("[dtb] uart:  {:#x} (size {:#x})", uart.base, uart.size),
    }

    // Unconditional, and outside every device's `if`. These counts say how much of
    // the tree we understood, which is what you want on an unfamiliar platform —
    // exactly when some other device is likely to be missing.
    println!(
        "[dtb] mmio:  {} windows, {} foreign RAM ranges",
        table.mmio.len(),
        table.foreign.len()
    );
    println!("[dtb] harts: {:?} (ids as reported)", table.hart_ids);
    match table.timebase_hz {
        Some(hz) => println!("[dtb] timebase: {hz} Hz"),
        None => println!("[dtb] timebase: absent (bounded waits will be skipped)"),
    }
    if table.disabled > 0 {
        println!(
            "[dtb] skipped {} node(s): status not okay, or below one that is not",
            table.disabled
        );
    }
}
