use crate::cpu::regs::GeneralRegs;

#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
pub struct UserContext {
    /// General registers
    pub general: GeneralRegs,
    /// Supervisor Status
    pub sstatus: usize,
    /// Supervisor Exception Program Counter
    pub sepc: usize,
}

impl UserContext {
    /// Get number of syscall
    pub fn get_syscall_num(&self) -> usize { self.general.a7 }

    /// Get return value of syscall
    pub fn get_syscall_ret(&self) -> usize { self.general.a0 }

    /// Set return value of syscall
    pub fn set_syscall_ret(&mut self, ret: usize) { self.general.a0 = ret; }

    /// Get syscall args
    pub fn get_syscall_args(&self) -> [usize; 6] {
        [
            self.general.a0,
            self.general.a1,
            self.general.a2,
            self.general.a3,
            self.general.a4,
            self.general.a5,
        ]
    }

    /// Set instruction pointer
    pub fn set_ip(&mut self, ip: usize) { self.sepc = ip; }

    /// Set stack pointer
    pub fn set_sp(&mut self, sp: usize) { self.general.sp = sp; }

    /// Get stack pointer
    pub fn get_sp(&self) -> usize { self.general.sp }

    /// Set tls pointer
    pub fn set_tls(&mut self, tls: usize) { self.general.tp = tls; }
}
