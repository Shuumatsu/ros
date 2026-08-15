//! Everything that is true of one instruction set and no other.
//!
//! Hart counts are absent by design: how many harts exist is a runtime fact, held as a
//! list of ids by `device_tree::hart_ids` and `cpu::secondary_hart_ids`.

pub mod riscv64;
