use super::table;
use crate::utils::ByteSize;

pub fn summary() {
    let Some(table) = table::get() else {
        println!("[dtb] not parsed");
        return;
    };

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
