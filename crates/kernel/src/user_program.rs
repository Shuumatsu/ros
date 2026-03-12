/// Hardcoded RISC-V user-space program.
///
/// Instructions:
///   li a7, 1       # syscall number = SYS_PRINT
///   li a0, 42      # argument = 42
///   ecall          # trap into kernel
///   li a7, 1       # syscall number = SYS_PRINT
///   li a0, 99      # argument = 99
///   ecall          # trap into kernel
///   li a7, 2       # syscall number = SYS_EXIT
///   ecall          # exit back to kernel
#[repr(align(4))]
pub struct AlignedProgram(pub [u32; 8]);

pub static USER_PROGRAM: AlignedProgram = AlignedProgram([
    0x00100893, // addi a7, x0, 1   (li a7, 1)
    0x02a00513, // addi a0, x0, 42  (li a0, 42)
    0x00000073, // ecall
    0x00100893, // addi a7, x0, 1   (li a7, 1)
    0x06300513, // addi a0, x0, 99  (li a0, 99)
    0x00000073, // ecall
    0x00200893, // addi a7, x0, 2   (li a7, 2)
    0x00000073, // ecall            (SYS_EXIT)
]);
