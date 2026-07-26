pub mod riscv64;

// There was a `pub const NCPU: usize = 8` here with no users. It was a third,
// disagreeing answer to "how many harts" — the linker reserves stack space for 16
// and `boot.S` parks the rest. The single source is `memory::stack::max_harts()`,
// which *derives* the count from that reserved area, so nothing can hold a stale
// copy. Anything needing a hart count should call it.
