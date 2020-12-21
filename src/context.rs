use riscv::register::*;

// #[derive(Debug)]
#[repr(C)]
pub struct TrapFrame {
    pub x: [usize; 32],            // General registers
    pub sstatus: sstatus::Sstatus, // Supervisor Status Register
    pub sepc: usize,               // Supervisor exception program counter
    pub stval: usize,              // Supervisor trap value
    pub scause: scause::Scause,    // Scause register: record the cause of exception/interrupt/trap
}
