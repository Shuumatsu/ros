pub mod riscv64;

// There was a `pub const NCPU: usize = 8` here with no users. It was a second,
// disagreeing answer to "how many harts", and the kernel does not have a compile-
// time one to give: the machine's harts are `device_tree::hart_ids()`, and the ones
// this kernel starts are `cpu::secondary_hart_ids()`. Both are lists, not counts,
// because a hart id is not a dense index — see `memory::stack`.
