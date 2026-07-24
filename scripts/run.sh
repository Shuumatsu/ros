#!/usr/bin/env bash
# Cargo runner for the kernel.
#
# Objcopies the freshly-built ELF into a flat RISC-V `Image` and boots it under
# OpenSBI. We boot the Image, NOT the ELF: once the kernel is linked at high
# virtual addresses (Phase 2) the ELF entry point is an unmapped high VA, while
# the flat Image is loaded at its physical `text_offset` and entered at `code0`
# (which relocates itself). Booting the Image also exercises the real Linux
# RISC-V Image boot protocol (header parsed by QEMU/the loader).
#
# Cargo invokes this as: run.sh <path-to-kernel-elf>
set -euo pipefail

ELF="$1"
IMG="${ELF}.bin"

llvm-objcopy -O binary "$ELF" "$IMG"

exec qemu-system-riscv64 \
    -s \
    -nographic \
    -no-reboot \
    -machine virt \
    -cpu rv64 \
    -d guest_errors,unimp \
    -smp 1 \
    -m 128M \
    -drive if=none,format=raw,file=hdd.dsk,id=foo \
    -device virtio-blk-device,drive=foo \
    -serial mon:stdio \
    -bios default \
    -device virtio-rng-device \
    -device virtio-gpu-device \
    -device virtio-net-device \
    -device virtio-tablet-device \
    -device virtio-keyboard-device \
    -kernel "$IMG"
