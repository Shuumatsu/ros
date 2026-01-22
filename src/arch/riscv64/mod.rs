pub mod paging;
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

#[inline(always)]
pub fn wait_forever() -> ! {
    loop {
        unsafe {
            riscv::asm::wfi();
        }
    }
}

#[inline(always)]
pub fn stack_pointer() -> usize {
    #[allow(unused_assignments)]
    let mut sp: usize = 0;

    unsafe {
        asm!("mv {0}, sp", out(reg) sp);
    }
    sp
}
