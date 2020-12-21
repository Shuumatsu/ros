// RVC uses a simple compression scheme that offers shorter 16-bit versions of common 32-bit RISC-V
// instructions when:
//     - the immediate or address offset is small
//     - one of the registers is the zero register (zero), the ABI link register (ra), or the ABI stack pointer (sp)
//     - the destination register and the first source register are identical
//     - the registers used are the 8 most popular ones.

// The C extension allows 16-bit instructions to be freely intermixed with 32-bit instructions, with the latter now able to start on any 16-bit boundary.
