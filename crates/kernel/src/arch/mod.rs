//! Everything that is true of one instruction set and no other, and the one name the rest
//! of the kernel reaches it by.
//!
//! The kernel says `arch::wait_forever()`, never `arch::riscv64::wait_forever()`. Which ISA
//! is a property of the build, fixed by `forced-target` in `Cargo.toml`, so spelling it at
//! a call site would put a build-wide constant into every file that parks a hart. The
//! re-export below is therefore the port interface: what this kernel requires of an
//! instruction set is exactly what appears in [`riscv64`]'s root and its four modules, and
//! nowhere else.
//!
//! That makes the boundary checkable. An ISA instruction outside this module, or a firmware
//! call outside [`riscv64::sbi`], is a leak — `grep` finds both.
//!
//! Hart counts are absent by design: how many harts exist is a runtime fact, held as a
//! list of ids by `device_tree::hart_ids` and paired with cpu slots by `cpu`.

#[cfg(target_arch = "riscv64")]
mod riscv64;

#[cfg(target_arch = "riscv64")]
pub use riscv64::*;
