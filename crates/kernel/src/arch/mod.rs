pub mod riscv64;

// No compile-time hart count belongs here: the machine's harts are
// `device_tree::hart_ids()` and the ones this kernel starts are
// `cpu::secondary_hart_ids()`, both lists rather than counts.
