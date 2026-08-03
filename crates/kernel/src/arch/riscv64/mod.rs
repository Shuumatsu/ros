pub mod interrupts;
pub mod sbi;

// Hart identity lives in `cpu`: `tp` points at a per-hart control block, and
// reading a field out of it is that module's business, not the ISA layer's.

/// Park this hart for good. The one parking primitive: `abort` and both `kmain`s
/// call it rather than open-coding the loop.
#[inline(always)]
pub fn wait_forever() -> ! {
    loop {
        riscv::asm::wfi();
    }
}
