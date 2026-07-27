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

    # ---- Zero the BSS, exactly once, by whichever hart gets here first ----
    # NOT "if hartid == 0". The firmware picks which hart enters the kernel and is
    # not required to pick 0 — OpenSBI runs a lottery, so at -smp 4 the winner
    # genuinely varies from boot to boot. Gating on a0 meant that on every boot
    # whose winner was not hart 0, the BSS was never cleared at all; it only looked
    # fine because QEMU hands out zeroed memory.
    #
    # So claim the job through a flag in .data — it cannot live in .bss, which is
    # the thing being cleared — and make every hart that loses wait for completion
    # before it can read a static. Flag: 0 = untouched, 1 = claimed, 2 = done.
    #
    # The claim MUST be conditional, which is why this is lr/sc and not amoswap.
    # An unconditional `amoswap.w 1` writes 1 whatever it finds, so a hart arriving
    # after the flag reached 2 clobbered the 2 back to 1 and then span forever
    # waiting for a 2 that could never return — deadlocking every secondary hart.
    # lr/sc stores only when nothing has touched the word since the load, so a hart
    # that observes 2 never writes at all.
    lla     t0, .Lbss_state
7:
    lr.w    t1, (t0)
    li      t2, 2
    beq     t1, t2, 3f          # already done: nothing to do, and nothing written
    bnez    t1, 7b              # claimed by someone still working: keep watching
    li      t2, 1
    sc.w    t2, t2, (t0)        # 0 -> 1, only if the word is still untouched
    bnez    t2, 7b              # lost the reservation: back to watching
    la      t2, _bss_start
    la      t3, _bss_end
    bgeu    t2, t3, 8f
2:
    sd      zero, (t2)
    addi    t2, t2, 8
    bltu    t2, t3, 2b
8:
    fence   w, w                # the zeroing must land before "done" is published
    li      t1, 2
    sw      t1, 0(t0)
3:
    fence   r, r                # do not read a static before the zeroing is visible
    # ---- Per-hart stack ----
    # Each hart's usable stack sits immediately above its own guard page, so its
    # top is _kernel_stack_start + stride * (hartid + 1). The stride is owned by
    # memory::stack and read from the word it exports; the hart count is *derived*
    # from the area the linker reserved, which is exactly how Rust derives it. So
    # neither side restates the other, and there is no constant here to drift.
    lla     t0, HART_STACK_STRIDE
    ld      t0, 0(t0)           # t0 = guard + stack, per hart
    la      t1, _kernel_stack_start
    la      t2, _kernel_stack_end
    sub     t2, t2, t1          # t2 = bytes reserved for all stacks
    divu    t2, t2, t0          # t2 = harts we have stack space for

    # A hartid at or beyond that has no stack. Continuing would put sp on another
    # hart's stack — or past the area entirely — and corrupt memory before Rust
    # runs, so there is nothing safe to do but park.
    bltu    tp, t2, 4f
5:
    wfi
    j       5b
4:
    addi    t3, tp, 1
    mul     t0, t0, t3
    add     sp, t1, t0          # sp = start + stride * (hartid + 1)

    # start(hartid = a0, dtb = a1, measured va_offset = a2)
    call    start

# BSS-zeroing claim flag: 0 = untouched, 1 = claimed, 2 = done. In .data so it is
# loaded as zero from the image and is NOT itself cleared by the loop it guards.
.section .data
.balign 4
.Lbss_state:
    .word   0
