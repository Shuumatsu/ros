use core::arch::global_asm;

global_asm!(
    r#"
    .option push
    .option norvc
    .option norelax

    .section .text.init.header, "ax"
    .global _start
    .type _start, @function
_start:
    j       {boot}
    .4byte  0
    .8byte  _text_offset
    .8byte  _image_size
    .8byte  0
    .4byte  0x00000002
    .4byte  0
    .8byte  0
    .8byte  0
    .4byte  0x05435352
    .4byte  0
    .global _image_header_end
_image_header_end:
    .size _start, . - _start

    .option pop
    "#,
    boot = sym super::entry::primary_entry,
);
