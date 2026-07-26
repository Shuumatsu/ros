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

    # ---- Install the early page table ----
    # The table is CONST-BUILT IN RUST (see memory::direct_map) and loaded as
    # ordinary data, so no PTE format, no RAM base and no translation mode
    # appears here — this code only points satp at it. Both symbols are reached
    # PC-relative: pre-paging the PC is physical and the image's VMA-LMA skew is
    # uniform, so `lla` yields their PHYSICAL addresses.
    lla     t0, EARLY_PGTABLE   # PA of the root table
    lla     t1, EARLY_SATP_TEMPLATE
    ld      t1, 0(t1)           # MODE=Sv39, ASID=0, PPN=0 — composed by `paging`
    srli    t2, t0, 12          # satp.PPN = the table's page number: the one
    or      t1, t1, t2          #   field a const cannot know (a link-time fact)
    csrw    satp, t1
    sfence.vma                  # new translation active; PC still physical,
                                # kept alive by the identity half of the map

    # ---- Jump high. The kernel is linked high, so the linker knows this
    # continuation's virtual address; subtracting the physical address we
    # actually reached it at yields the real VA - PA skew. That goes to start()
    # purely as a CROSS-CHECK against direct_map::VA_OFFSET (which the linker
    # cannot read) — translation itself no longer depends on measuring it. ----
    lla     t0, .Lhigh_entry
    ld      t1, 0(t0)           # t1 = VMA(1f)  (linker-provided absolute high VA)
    lla     t0, 1f              # t0 = PMA(1f)  (where we actually are)
    sub     a2, t1, t0          # a2 = measured VA - PA  -> start()'s 3rd arg
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

    # start(hartid = a0, dtb = a1, measured va_offset = a2)
    call    start
