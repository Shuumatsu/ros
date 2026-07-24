# No compressed instructions (keeps code0 a fixed 4-byte jump), and no linker
# relaxation (the early code runs before gp is set, so nothing may be turned
# into gp-relative accesses).
.option norvc
.option norelax

.section .text.init

# ---------------------------------------------------------------------------
# RISC-V Linux Image header (64 bytes). A loader following the boot protocol
# enters the image at its start (offset 0 = code0), so code0 jumps over the
# header to the real entry. Layout per Documentation/riscv/boot-image-header.rst.
# ---------------------------------------------------------------------------
.global _start
_start:
    j       _boot               # 0x00  code0: jump past the header
    .4byte  0                   # 0x04  code1
    .8byte  _text_offset        # 0x08  text_offset (from kernel.ld, single source)
    .8byte  _image_size         # 0x10  image_size: static memory footprint
    .8byte  0                   # 0x18  flags: bit0 = endianness (0 = little)
    .4byte  0x00000002          # 0x20  version 0.2
    .4byte  0                   # 0x24  res1
    .8byte  0                   # 0x28  res2
    .8byte  0                   # 0x30  magic (deprecated in v0.2, 0)
    .4byte  0x05435352          # 0x38  magic2 = "RSC\x05"
    .4byte  0                   # 0x3C  res3 (PE COFF header offset)

.global _boot
_boot:
    # Entered in S-mode by OpenSBI: a0 = hartid, a1 = dtb. We execute at the
    # PHYSICAL load address (~0x8020_0040), but the kernel is LINKED at high VAs,
    # so until paging is on we may only use PC-relative addressing (lla) — never
    # an absolute symbol load. a0/a1 are preserved untouched until `call start`.
    mv      tp, a0              # hart_id() reads tp

    # ---- Build a minimal early page table: three Sv39 gigapages (1 GiB each) ----
    #   root[0]    VA [0,1G)     -> PA [0,1G)    identity: MMIO devices
    #   root[ram]  VA [ram,+1G)  -> PA [ram,+1G) identity: RAM (kernel, heap, dtb)
    #   root[256]  VA [high,+1G) -> PA [ram,+1G) the kernel's high-half home
    # The RAM base and its root index are DERIVED from where we actually run
    # (round _start down to 1 GiB) — no hardcoded RAM address. Leaf gigapage
    # PTE = (PA>>12 << 10) | V|R|W|X|A|D. (This 1 GiB window covers the QEMU
    # sizes we run; >1 GiB RAM would need more entries.)
    lla     t0, early_pgtable   # physical address of the table (PC-relative)

    lla     t2, _start
    srli    t2, t2, 30
    slli    t2, t2, 30          # t2 = RAM base, 1 GiB-aligned
    srli    t3, t2, 2           # t3 = (base>>12)<<10 ...
    ori     t3, t3, 0xcf        #      | V R W X A D = RAM gigapage leaf PTE
    srli    t4, t2, 30          # t4 = identity root index (RAM base >> 30)

    li      t1, 0xcf            # root[0]: identity low 1 GiB (MMIO devices), PA 0
    sd      t1, 0(t0)

    slli    t5, t4, 3           # root[ram]: identity RAM
    add     t5, t0, t5
    sd      t3, 0(t5)

    li      t5, 8*256           # root[256]: high-half home (bottom of the Sv39
    add     t5, t0, t5          #            high half), same frame as RAM
    sd      t3, 0(t5)

    # ---- Enable Sv39 translation ----
    srli    t1, t0, 12          # PPN of the table
    li      t2, 8
    slli    t2, t2, 60          # MODE = 8 (Sv39)
    or      t1, t1, t2
    csrw    satp, t1
    sfence.vma                  # new translation active; PC still physical,
                                # kept alive by the RAM identity mapping

    # ---- Derive the VA offset, then jump high. The kernel is linked high, so
    # the linker knows this continuation's virtual address; subtracting its
    # physical address (where we actually run) yields VA - PA. We hand that to
    # start() so the offset is never hardcoded in Rust or asm — it is measured
    # from the linked layout vs. the real load address. ----
    lla     t0, .Lhigh_entry
    ld      t1, 0(t0)           # t1 = VMA(1f)  (linker-provided absolute high VA)
    lla     t0, 1f              # t0 = PMA(1f)  (where we actually are)
    sub     a2, t1, t0          # a2 = va_offset = VMA - PMA  -> start()'s 3rd arg
    jr      t1                  # enter the high half
.balign 8
.Lhigh_entry:
    .dword  1f
1:
    # Now executing at HIGH VAs: PC-relative lla yields high addresses, and the
    # absolute references in Rust resolve correctly.

    la      gp, _global_pointer # high gp (norelax above keeps this absolute-safe)

    # Boot hart (hartid 0) zeroes the BSS.
    bnez    a0, 3f
    la      t0, _bss_start
    la      t1, _bss_end
    bgeu    t0, t1, 3f
2:
    sd      zero, (t0)
    addi    t0, t0, 8
    bltu    t0, t1, 2b
3:
    # Per-hart stack: sp = _kernel_stack_start + 64 KiB * (hartid + 1).
    la      sp, _kernel_stack_start
    li      t0, 64*1024
    addi    t1, tp, 1
    mul     t0, t0, t1
    add     sp, sp, t0

    # start(hartid = a0, dtb = a1)
    call    start

# 4 KiB-aligned, zero-filled early page table. Lives in .data (PROGBITS) so it
# is loaded pre-zeroed from the image and reachable physically before paging.
.section .data
.align 12
early_pgtable:
    .space 4096
