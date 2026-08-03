pub mod interrupts;
pub mod sbi;

use core::arch::asm;

#[inline(always)]
pub fn hart_id() -> usize {
    let hart_id: usize;
    unsafe {
        asm!("mv {0}, tp", out(reg) hart_id, options(nomem, nostack));
    }
    hart_id
}

/// Park this hart for good. The one parking primitive: `abort` and both `kmain`s
/// call it rather than open-coding the loop.
#[inline(always)]
pub fn wait_forever() -> ! {
    loop {
        riscv::asm::wfi();
    }
}
