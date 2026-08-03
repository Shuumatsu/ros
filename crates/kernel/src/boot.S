# No compressed instructions (keeps code0 a fixed 4-byte jump), and no linker
# relaxation (this runs before gp is set, so nothing may become gp-relative).
.option norvc
.option norelax

# lr.w/sc.w below need the A extension. Not redundant with the riscv64imac target: the
# feature string reaching a module-level `global_asm!` depends on codegen-unit
# partitioning, and at -C opt-level=3 it omits A — release then fails with
# `instruction requires the following: 'Zalrsc'` while debug compiles happily.
.option arch, +a

.section .text.init

# ---------------------------------------------------------------------------
# RISC-V Linux Image header (64 bytes), per Documentation/riscv/boot-image-header.rst.
# A loader enters the image at offset 0, so code0 jumps past the header.
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

# ---------------------------------------------------------------------------
# Two entry points, one shared body. The boot hart arrives at _boot with the device
# tree in a1; every other hart arrives at _secondary_start because the boot hart asked
# SBI HSM to start it there. Both must install the early page table and relocate high
# first, so that is written once below and t6 records which path to take afterwards.
#
# Nothing here is indexed by hart id; neither path computes an address from a0. See
# memory::stack's module docs for the rule.
# ---------------------------------------------------------------------------
.global _boot
_boot:
    # Entered in S-mode by OpenSBI: a0 = hartid, a1 = dtb.
    li      t6, 0               # t6 = 0: take the boot-hart path below
    j       _enable_paging

.global _secondary_start
_secondary_start:
    # Entered in S-mode by SBI HSM: a0 = hartid, a1 = opaque, which the boot hart set
    # to the VIRTUAL address of this hart's cpu::Cpu block (cpu::start_secondaries).
    li      t6, 1               # t6 = 1: take the secondary path below

_enable_paging:
    # We execute at the PHYSICAL load address (~0x8020_0040) but the kernel is LINKED
    # high, so until paging is on only PC-relative addressing (lla) is valid — never an
    # absolute symbol load. a0/a1 stay untouched until the call at the end.
    #
    # tp is not set here: it points at this hart's cpu::Cpu block, and neither
    # KERNEL_CPUS nor a secondary's block is reachable until we are running high.

    # ---- A trap vector, before anything can trap ----
    # stvec is UNDEFINED at hart entry, so a fault before this point jumps to whatever
    # the register happens to hold. Point it at a parking loop instead, which stops the
    # hart with sepc/scause/stval intact.
    #
    # Physical, because that is what the PC is. Re-pointed at the high alias after the
    # jump below, since the kernel table has no identity mapping. The low two bits of
    # stvec are MODE, so the target must be 4-byte aligned; 0 = Direct.
    lla     t0, _trap_park
    csrw    stvec, t0

    # ---- Install the early page table ----
    # The table is const-built in Rust (memory::direct_map) and loaded as ordinary
    # data, so no PTE format, RAM base or translation mode appears here — this only
    # points satp at it. Pre-paging the PC is physical and the image's VMA-LMA skew is
    # uniform, so `lla` yields both PHYSICAL addresses.
    lla     t0, EARLY_PGTABLE   # PA of the root table
    lla     t1, EARLY_SATP_TEMPLATE
    ld      t1, 0(t1)           # MODE=Sv39, ASID=0, PPN=0 — composed by `paging`
    srli    t2, t0, 12          # satp.PPN = the table's page number: the one
    or      t1, t1, t2          #   field a const cannot know (a link-time fact)
    csrw    satp, t1
    sfence.vma                  # new translation active; PC still physical,
                                # kept alive by the identity half of the map

    # ---- Jump high ----
    # The linker knows this continuation's virtual address; subtracting the physical
    # address we actually reached it at yields the real VA - PA skew. That goes to
    # start() purely as a cross-check against direct_map::VA_OFFSET, which the linker
    # cannot read — translation no longer depends on measuring it.
    lla     t0, _high_entry_ptr
    ld      t1, 0(t0)           # t1 = VMA(_high_entry)  (linker-provided high VA)
    lla     t0, _high_entry     # t0 = PMA(_high_entry)  (where we actually are)
    sub     a2, t1, t0          # a2 = measured VA - PA  -> start()'s 3rd arg
    jr      t1                  # enter the high half
.balign 8
_high_entry_ptr:
    .dword  _high_entry
_high_entry:
    la      gp, _global_pointer # high gp (norelax above keeps this absolute-safe)

    # The same parking loop, now by its linked address. The physical one dies with the
    # identity mapping when a secondary installs the kernel table, so it has to be
    # re-pointed while both still resolve.
    la      t0, _trap_park
    csrw    stvec, t0

    bnez    t6, _secondary_path

# ---------------------------------------------------------------------------
# Boot hart. Owns the one static stack, and therefore the one-time setup.
# ---------------------------------------------------------------------------
    # ---- Claim the boot role, exactly once ----
    # There is exactly one boot stack and nothing here can allocate another. The SBI
    # boot protocol leaves every other hart STOPPED for hart_start, but a loader that
    # released all of them would otherwise put every hart on the same sp.
    #
    # Not "if hartid == 0": the firmware's choice of entry hart is its own.
    #
    # The flag lives in .data, not .bss — .bss is what the winner is about to zero.
    #
    # lr/sc rather than amoswap because the claim must be CONDITIONAL; an unconditional
    # swap writes 1 over whatever it finds and cannot report a loss.
    lla     t0, _boot_claim
_claim_loop:
    lr.w    t1, (t0)
    bnez    t1, _park           # taken: another hart owns the boot stack
    li      t2, 1
    sc.w    t2, t2, (t0)
    bnez    t2, _claim_loop     # spurious failure or a racing hart: look again

    # ---- Zero the BSS ----
    # Uncontended: a hart that loses the claim parks at _park without reaching here, so
    # nothing else can touch .bss while this runs.
    la      t0, _bss_start
    la      t1, _bss_end
    bgeu    t0, t1, _bss_done
_bss_loop:
    sd      zero, (t0)
    addi    t0, t0, 8
    bltu    t0, t1, _bss_loop
_bss_done:

    # The one static stack, taken whole. Its top is the end of the reserved area,
    # because memory::stack puts the guard page at the bottom.
    la      sp, _boot_stack_end

    # ---- This hart's control block ----
    # Slot 0 of KERNEL_CPUS is the boot hart's, so the array symbol is the block
    # address. cpu::hart_id() reads the field stored here, and it has to be right
    # before the first println!, well before start().
    la      tp, KERNEL_CPUS
    sd      a0, 8(tp)           # Cpu.hartid — offset asserted in cpu/mod.rs

    # start(hartid = a0, dtb = a1, measured va_offset = a2)
    call    start
    # start() is `-> !`. Stop deterministically rather than fall through: the next
    # label is the SECONDARY path, which does `mv sp, a1` — and on the boot hart a1
    # still holds the DTB pointer.
    j       _park

# ---------------------------------------------------------------------------
# Secondary hart. Its stack was allocated and mapped by the boot hart.
# ---------------------------------------------------------------------------
_secondary_path:
    # ---- Adopt the kernel page table before touching the stack ----
    # a1 is an address only the kernel table maps, so there is no stack to run Rust on
    # until this is done. memory::kernel_table explains why the stacks live where they
    # do and why no handshake is needed here.
    la      t0, KERNEL_SATP
    ld      t0, 0(t0)
    fence   r, rw               # acquire, pairing with the release store in
                                # kernel_table::init: the tree the boot hart built must
                                # be visible before the walker reads it. The sfence
                                # below orders the walker only against THIS hart's
                                # memory operations, so it cannot substitute.
    csrw    satp, t0
    sfence.vma                  # identity mapping is gone from here on; the PC is
                                # high, which the kernel table maps

    # ---- This hart's control block, and the stack inside it ----
    # a1 is the SBI `opaque`: the address of this hart's cpu::Cpu block, already filled
    # in. stack_top is its first field, which is what lets one opaque word carry both.
    # Offsets are asserted in cpu/mod.rs.
    mv      tp, a1
    ld      sp, 0(tp)           # Cpu.stack_top
    sd      a0, 8(tp)           # Cpu.hartid

    # secondary_start(hartid = a0)
    call    secondary_start
    # Also `-> !`. Stated so a label added here cannot silently become the landing site.
    j       _park

# A hart with nowhere to run: only reachable by a second arrival at _boot, i.e. a
# loader that ignored the SBI boot protocol. There is no stack for it and no way to
# make one here.
#
# It stays idle — the firmware already considers it STARTED, so start_secondaries finds
# it in that state and reports it rather than trying to start it.
.balign 4
_park:
    wfi
    j       _park

# A trap with no handler behind it — see the stvec write above. Today that means *any*
# trap, since the trap subsystem is parked in ../attic/trap/ and nothing overwrites
# stvec later. A separate loop from _park on purpose: the PC alone then says which of
# the two happened, which matters in a boot path that has no console.
.balign 4
_trap_park:
    wfi
    j       _trap_park

# Boot-role claim flag: 0 = free, 1 = taken. In .data so it is loaded as zero from the
# image and is NOT itself cleared by the loop it guards.
.section .data
.balign 4
_boot_claim:
    .word   0

