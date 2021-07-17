# https://seisman.github.io/how-to-write-makefile/introduction.html

MKFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
MAKEFILE_DIR := $(abspath $(dir $(MKFILE_PATH)))

# Binutils
OBJDUMP := rust-objdump --arch-name=riscv64
OBJCOPY := rust-objcopy --binary-architecture=riscv64

# BOARD
BOOTLOADER := $(MAKEFILE_DIR)/bootloader/rustsbi-qemu.bin

# KERNEL ENTRY
KERNEL_ENTRY_PA := 0x80200000

# Building
TARGET := riscv64gc-unknown-none-elf
MODE := release

TARGET_DIR := $(MAKEFILE_DIR)/target/$(TARGET)/$(MODE)

KERNEL_PROJ_DIR := $(MAKEFILE_DIR)/kernel
KERNEL_ELF := $(TARGET_DIR)/kernel
KERNEL_BIN := $(TARGET_DIR)/kernel.bin

USER_LIB_PROJ_DIR := $(MAKEFILE_DIR)/user_lib

USER_APPS_PROJ_DIR := $(MAKEFILE_DIR)/user_apps
USER_APPS_BIN_DIR := $(MAKEFILE_DIR)/user_apps/bin
USER_APPS_BIN_SRCS := $(wildcard $(USER_APPS_BIN_DIR)/*.rs)
USER_APPS_ELFS := $(patsubst $(USER_APPS_BIN_DIR)/%.rs, $(TARGET_DIR)/%, $(USER_APPS_BIN_SRCS))
USER_APPS_BINS := $(patsubst $(USER_APPS_BIN_DIR)/%.rs, $(TARGET_DIR)/%.bin, $(USER_APPS_BIN_SRCS))

env:
	(rustup target list | grep "$(TARGET) (installed)") || rustup target add $(TARGET)
	cargo install cargo-binutils --vers ~0.2
	rustup component add rust-src
	rustup component add llvm-tools-preview


user-lib: 
	cd $(USER_LIB_PROJ_DIR) && \
	cargo build --release
	
user-apps: 
	cd $(USER_APPS_PROJ_DIR) && \
	cargo build --release && \
	$(foreach elf, $(USER_APPS_ELFS), $(OBJCOPY) $(elf) --strip-all -O binary $(patsubst $(TARGET_DIR)/%, $(TARGET_DIR)/%.bin, $(elf));)

kernel-binary: 
	cd $(KERNEL_PROJ_DIR) && \
	cargo build --release && \
	$(OBJCOPY) $(KERNEL_ELF) --strip-all -O binary $(KERNEL_BIN)

build: user-lib user-apps kernel-binary

pure-run:
	qemu-system-riscv64 \
		-s \
		-smp 4 \
		-machine virt \
		-nographic \
		-bios $(BOOTLOADER) \
		-device loader,file=$(KERNEL_BIN),addr=$(KERNEL_ENTRY_PA)

run: build pure-run

debug: build
	@qemu-system-riscv64 \
		-s -S \
		-smp 4 \
		-machine virt \
		-nographic \
		-bios $(BOOTLOADER) \
		-device loader,file=$(KERNEL_BIN),addr=$(KERNEL_ENTRY_PA)

clean:
	cargo clean

.PHONY: clean debug run build kernel-binary user-apps user-lib env  