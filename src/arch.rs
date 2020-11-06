// use volatile::Volatile;

#[inline(always)]
pub fn hart_id() -> usize {
    #[allow(unused_assignments)]
    let mut hart_id: usize = 0;

    unsafe {
        // llvm_asm!("mv $0, tp" : "=r"(hart_id) ::: "volatile");
        asm!("mv {0}, tp", out(reg) hart_id);
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
