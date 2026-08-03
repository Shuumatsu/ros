# No compressed instructions (keeps code0 a fixed 4-byte jump), and no linker
# relaxation (the early code runs before gp is set, so nothing may be turned
# into gp-relative accesses).
.option norvc
.option norelax

# The boot-role claim below uses lr.w/sc.w, so this file needs the A extension.
#
# DO NOT DELETE as redundant. The target (riscv64imac) does declare A, but the
# feature string reaching the assembler for a module-level `global_asm!` depends on
# codegen-unit partitioning, and at -C opt-level=3 it does not include it: without
# this line the RELEASE build fails with `instruction requires the following:
# 'Zalrsc'` while the debug build compiles happily.
.option arch, +a

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

# ---------------------------------------------------------------------------
# Two entry points, one shared body.
#
# The boot hart arrives at _boot, chosen by the firmware, with the device tree
# in a1. Every other hart arrives at _secondary_start because the boot hart
# asked SBI HSM to start it there, with its stack in a1. Both have to install
# the early page table and relocate to high virtual addresses first, so that is
# written once below and t6 records which path to take afterwards.
#
# NOTHING HERE IS INDEXED BY HART ID. Neither path computes an address from a0.
# Why that rule exists, and what it cost to learn: memory::stack's module docs.
# ---------------------------------------------------------------------------
.global _boot
_boot:
    # Entered in S-mode by OpenSBI: a0 = hartid, a1 = dtb.
    li      t6, 0               # t6 = 0: take the boot-hart path below
    j       .Lenable_paging

.global _secondary_start
_secondary_start:
    # Entered in S-mode by SBI HSM: a0 = hartid, a1 = opaque, which the boot hart
    # set to the VIRTUAL address of this hart's stack top (cpu::start_secondaries).
    li      t6, 1               # t6 = 1: take the secondary path below

.Lenable_paging:
    # We execute at the PHYSICAL load address (~0x8020_0040), but the kernel is
    # LINKED at high VAs, so until paging is on we may only use PC-relative
    # addressing (lla) — never an absolute symbol load. a0/a1 are preserved
    # untouched until the call at the end.
    # tp is NOT set here. It holds a pointer to this hart's cpu::Cpu block, and both
    # KERNEL_CPUS and a secondary's block address are only reachable once paging is
    # on and we are running high. Each path sets it after the jump; nothing between
    # here and there reads it.

    # ---- A trap vector, before anything can trap ----
    # stvec is UNDEFINED at hart entry: the firmware does not set it, so an exception
    # before this point jumps to whatever the register happens to hold. Point it at a
    # parking loop: a fault then stops the hart deterministically with
    # sepc/scause/stval intact for a debugger. Linux does the same, first thing, in
    # secondary_start_sbi.
    #
    # This is currently the ONLY trap vector the kernel installs — the trap subsystem
    # is parked in ../attic/trap/, so nothing overwrites stvec later.
    #
    # PHYSICAL, because that is what the PC is — `lla` pre-paging yields the load
    # address. Re-pointed at the high alias after the jump below, since the kernel
    # table has no identity mapping and this address stops resolving there.
    #
    # The low two bits of stvec are MODE, so the target must be 4-byte aligned;
    # .Ltrap_park is declared `.balign 4` for that reason. 0 = Direct.
    lla     t0, .Ltrap_park
    csrw    stvec, t0

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
    ld      t1, 0(t0)           # t1 = VMA(.Lhigh)  (linker-provided high VA)
    lla     t0, .Lhigh          # t0 = PMA(.Lhigh)  (where we actually are)
    sub     a2, t1, t0          # a2 = measured VA - PA  -> start()'s 3rd arg
    jr      t1                  # enter the high half
.balign 8
.Lhigh_entry:
    .dword  .Lhigh
.Lhigh:
    # Now executing at HIGH VAs: PC-relative lla yields high addresses, and the
    # absolute references in Rust resolve correctly.

    la      gp, _global_pointer # high gp (norelax above keeps this absolute-safe)

    # The same parking loop, now named by its linked address. The physical one set
    # above dies with the identity mapping when a secondary installs the kernel
    # table, so it has to be re-pointed while both still resolve.
    la      t0, .Ltrap_park
    csrw    stvec, t0

    bnez    t6, .Lsecondary

# ---------------------------------------------------------------------------
# Boot hart. Owns the one static stack, and therefore the one-time setup.
# ---------------------------------------------------------------------------
    # ---- Claim the boot role, exactly once ----
    # There is exactly ONE boot stack and nothing here can allocate another, so
    # only one hart may continue. The SBI boot protocol says the firmware enters
    # the kernel on a single hart and leaves the rest STOPPED for hart_start, but
    # a loader that released all of them would otherwise put every hart on the
    # same sp — and the resulting corruption would be untraceable.
    #
    # NOT "if hartid == 0" — the firmware's choice of entry hart is its own, and
    # cpu::record_boot_hart says what assuming otherwise cost. This is the election;
    # that function only records the winner.
    #
    # The flag lives in .data, not .bss: .bss is what the winner is about to zero.
    #
    # lr/sc rather than amoswap because the claim must be CONDITIONAL — an
    # unconditional swap writes 1 over whatever it finds and cannot report a loss.
    # The retry is not optional either: an sc may fail spuriously, and only the
    # re-read distinguishes that from a genuine loss.
    lla     t0, .Lboot_claim
.Lclaim:
    lr.w    t1, (t0)
    bnez    t1, .Lpark          # taken: another hart owns the boot stack
    li      t2, 1
    sc.w    t2, t2, (t0)
    bnez    t2, .Lclaim         # spurious failure or a racing hart: look again

    # ---- Zero the BSS ----
    # Uncontended, and not because no other hart exists — the claim above exists
    # precisely because one might. It is that a hart which loses parks at .Lpark
    # without ever reaching here, so nothing else can read or write .bss while this
    # runs. Rust may therefore touch a static the instant it is entered.
    la      t0, _bss_start
    la      t1, _bss_end
    bgeu    t0, t1, .Lbss_done
.Lbss_loop:
    sd      zero, (t0)
    addi    t0, t0, 8
    bltu    t0, t1, .Lbss_loop
.Lbss_done:

    # The one static stack, taken whole. No arithmetic and no hart id: its top is
    # the end of the reserved area, because memory::stack puts the guard page at
    # the bottom.
    la      sp, _boot_stack_end

    # ---- This hart's control block ----
    # Slot 0 of KERNEL_CPUS is the boot hart's by definition, so the array symbol is
    # the block address. Record a0 in it: cpu::hart_id() reads that field, and it has
    # to be right before the first println!, which happens well before start().
    la      tp, KERNEL_CPUS
    sd      a0, 8(tp)           # Cpu.hartid — offset asserted in cpu/mod.rs

    # start(hartid = a0, dtb = a1, measured va_offset = a2)
    call    start
    # start() is `-> !`, so reaching here means something is already badly wrong.
    # Stop deterministically rather than fall through: the next label is the
    # SECONDARY path, which re-writes satp and then does `mv sp, a1` — and on the
    # boot hart a1 still holds the DTB POINTER, so it would run secondary_start on a
    # stack pointed into the device tree blob.
    j       .Lpark

# ---------------------------------------------------------------------------
# Secondary hart. Its stack was allocated and mapped by the boot hart.
# ---------------------------------------------------------------------------
.Lsecondary:
    # ---- Adopt the kernel page table before touching the stack ----
    # It has to happen in assembly, and it has to happen before sp: a1 is an address
    # only the kernel table maps, so there is no stack to run Rust on until this is
    # done. memory::kernel_table explains why the stacks live where they do and why
    # no handshake is needed here.
    la      t0, KERNEL_SATP
    ld      t0, 0(t0)
    fence   r, rw               # acquire, pairing with the release store in
                                # kernel_table::init: the tree the boot hart built
                                # must be visible before the walker reads it. The
                                # sfence below only orders the walker against THIS
                                # hart's memory operations, so it cannot substitute.
    csrw    satp, t0
    sfence.vma                  # identity mapping is gone from here on; the PC is
                                # high, which the kernel table maps

    # ---- This hart's control block, and the stack inside it ----
    # a1 is the SBI `opaque` the boot hart passed: the address of this hart's
    # cpu::Cpu block, already filled in. stack_top is its first field, which is what
    # lets one opaque word carry both. Offsets are asserted in cpu/mod.rs.
    mv      tp, a1
    ld      sp, 0(tp)           # Cpu.stack_top
    sd      a0, 8(tp)           # Cpu.hartid

    # secondary_start(hartid = a0)
    call    secondary_start
    # Also `-> !`. The fall-through would land on .Lpark anyway; stating it means the
    # next label added here cannot silently become the landing site.
    j       .Lpark

# A hart with nowhere to run. Only reachable by a second arrival at _boot, i.e.
# a loader that ignored the SBI boot protocol and released every hart into the
# kernel. There is no stack for it and no way to make one here, so it idles.
#
# It stays idle: the firmware already considers it STARTED, so start_secondaries
# will find it in that state and report it rather than trying to start it. That
# is a real loss of a hart on such a platform, and the honest price of refusing
# to let two harts share one stack.
.balign 4
.Lpark:
    wfi
    j       .Lpark

# A trap with no handler behind it — see the stvec write at the top. Today that means
# *any* trap, since the trap subsystem is parked. A separate loop from .Lpark on
# purpose, even though the two do the same thing: the PC alone then says which of the
# two happened, which matters in a boot path that has no console. scause/sepc/stval
# describe the fault itself.
.balign 4
.Ltrap_park:
    wfi
    j       .Ltrap_park

# Boot-role claim flag: 0 = free, 1 = taken. In .data so it is loaded as zero
# from the image and is NOT itself cleared by the loop it guards.
.section .data
.balign 4
.Lboot_claim:
    .word   0
