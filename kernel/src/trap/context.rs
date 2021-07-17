use core::mem::size_of;

use riscv::register::sstatus::{self, Sstatus, SPP};

use crate::cpu::GeneralRegs;

#[repr(C)]
#[derive(Debug)]
pub struct TrapContext {
    pub general: GeneralRegs,
    pub sstatus: Sstatus,
    pub sepc: usize,
}
const_assert_eq!(size_of::<TrapContext>(), 34 * 8);

impl TrapContext {
    pub fn set_sp(&mut self, sp: usize) {
        self.general.sp = sp;
    }

    pub fn app_init_context(entry: usize, sp: usize) -> Self {
        let mut sstatus = sstatus::read();
        sstatus.set_spp(SPP::User);
        let mut cx = Self {
            general: GeneralRegs::default(),
            sstatus,
            sepc: entry,
        };
        cx.set_sp(sp);
        cx
    }
}
