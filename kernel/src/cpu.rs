use riscv::register::sstatus;

#[derive(Debug, Clone, Copy)]
pub struct CPU {
    pub hart_id: usize,
}

#[inline(always)]
pub fn examine_cpu() -> CPU {
    let mut ptr: usize = 0;

    unsafe {
        llvm_asm!("mv $0, tp" : "=r"(ptr) ::: "volatile");
        *(ptr as *const _)
    }
}

pub fn without_interrupts<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    let sie = sstatus::read().sie();

    unsafe {
        sstatus::clear_sie();
    }

    let r = f();

    if sie {
        unsafe {
            sstatus::set_sie();
        }
    }

    r
}
