# ---- Config -----------------------------------------------------------------

ARCH ?= x86_64

ifeq ($(ARCH),x86_64)
  TARGET := x86_64-unknown-none
  QEMU   := qemu-system-x86_64
endif
ifeq ($(ARCH),aarch64)
  TARGET := aarch64-unknown-none-softfloat
  QEMU   := qemu-system-aarch64
endif
ifeq ($(filter x86_64 aarch64,$(ARCH)),)
  $(error ARCH=$(ARCH) is not supported, use ARCH=x86_64 or ARCH=aarch64)
endif

# Read OS version directly from kernel/Cargo.toml
VERSION := $(shell sed -n '/^\[package\]/,/^\[/p' kernel/Cargo.toml | sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1)
ifeq ($(VERSION),)
  VERSION := unknown
endif

BUILD_DIR  := build

LLD        := /opt/homebrew/bin/ld.lld
OBJCOPY    := /opt/homebrew/opt/llvm/bin/llvm-objcopy
CLANG      := /opt/homebrew/opt/llvm/bin/clang

KERNEL_SRC := $(shell find kernel/src -name '*.rs')
KERNEL_LIB := kernel/target/$(TARGET)/release/libzodiac_os.a

.DEFAULT_GOAL := all
.PHONY: all clean clean-disk run run-hvf run-debug watch dirs iso iso-inspect

dirs:
	@mkdir -p $(BUILD_DIR)

# ---- Kernel (Rust), shared by every architecture ------------------------------
$(KERNEL_LIB): $(KERNEL_SRC) kernel/Cargo.toml
	@echo "[RUST] Building kernel (release, $(TARGET))..."
	cd kernel && cargo +nightly build --release \
		-Z build-std=core,compiler_builtins,alloc \
		-Z build-std-features=compiler-builtins-mem \
		--target $(TARGET)

# =============================================================================
#  x86_64: BIOS boot sector + flat binary on a raw disk image
# =============================================================================
ifeq ($(ARCH),x86_64)

BOOT_ASM      := bootloader/x86_64/boot.asm
BOOT_INCLUDES := $(filter-out $(BOOT_ASM),$(wildcard bootloader/x86_64/*.asm))
LINKER        := kernel/linker.ld

BOOT_BIN   := $(BUILD_DIR)/boot.bin
KERNEL_ELF := $(BUILD_DIR)/kernel.elf
KERNEL_BIN := $(BUILD_DIR)/kernel.bin
DISK_IMG   := $(BUILD_DIR)/zodiacos.img

ISO_IMG    := $(BUILD_DIR)/zodiacos-v$(VERSION)-$(ARCH).iso
ISO_ROOT   := $(BUILD_DIR)/iso

# Data disk lives outside build/ so `make clean` never deletes your files
DATA_DIR   := disk
DATA_IMG   := $(DATA_DIR)/data.img

# Must match KERNEL_SECTORS in bootloader/x86_64/boot.asm (640 sectors * 512 bytes)
MAX_KERNEL_BYTES := 327680

# COM1 output (slog!/log_info! in the kernel) goes to this terminal.
# To keep the terminal quiet and read logs later, use instead:
#   -serial file:$(BUILD_DIR)/serial.log
QEMU_DRIVES := \
	-m 256M \
	-rtc base=localtime \
	-serial stdio \
	-drive format=raw,file=$(DISK_IMG),if=ide,index=0 \
	-drive format=raw,file=$(DATA_IMG),if=ide,index=1

all: $(DISK_IMG) $(DATA_IMG)
	@echo ""
	@echo "  Build complete! Disk image: $(DISK_IMG)"
	@echo "  Run with:  make run"
	@echo ""

$(BOOT_BIN): $(BOOT_ASM) $(BOOT_INCLUDES) | dirs
	@echo "[ASM]  Assembling Bootloader ($(ARCH))..."
	nasm -f bin -I bootloader/$(ARCH)/ -o $@ $<
	@test $$(wc -c < $@) -eq 512 || (echo "ERROR: Bootloader is not 512 bytes!" && exit 1)
	@echo "       $$(wc -c < $@) bytes — OK"
	@printf "       Boot signature: "; xxd -s 510 -l 2 $@

$(KERNEL_ELF): $(KERNEL_LIB) $(LINKER) | dirs
	@echo "[LD]   Linking kernel ELF..."
	$(LLD) -u _start -T $(LINKER) -o $@ $(KERNEL_LIB)

$(KERNEL_BIN): $(KERNEL_ELF)
	@echo "[OBJ]  Extracting flat binary..."
	$(OBJCOPY) -O binary $< $@
	@echo "       $$(wc -c < $@) bytes (flat binary)"
	@test $$(wc -c < $@) -le $(MAX_KERNEL_BYTES) || \
		(echo "ERROR: kernel exceeds $(MAX_KERNEL_BYTES) bytes. Raise KERNEL_SECTORS in $(BOOT_ASM) and MAX_KERNEL_BYTES here." && exit 1)

$(DISK_IMG): $(BOOT_BIN) $(KERNEL_BIN)
	@echo "[IMG]  Creating disk image..."
	dd if=/dev/zero     of=$@ bs=1m  count=64 status=none
	dd if=$(BOOT_BIN)   of=$@ bs=512 seek=0 conv=notrunc status=none
	dd if=$(KERNEL_BIN) of=$@ bs=512 seek=1 conv=notrunc status=none
	@echo "       Disk image ready: $@"

# Data disk (created once, never rebuilt)
$(DATA_IMG):
	@echo "[DATA] Creating 16 MB data disk..."
	@mkdir -p $(DATA_DIR)
	dd if=/dev/zero of=$@ bs=1m count=16 status=none

# NOTE: this ISO is not bootable yet (no El Torito boot record).
iso: $(DISK_IMG)
	@echo "[ISO]  Creating ZodiacOS v$(VERSION) ISO for $(ARCH)..."
	@mkdir -p $(ISO_ROOT)
	cp $(DISK_IMG) $(ISO_ROOT)/zodiacos.img
	xorriso -as mkisofs \
		-V ZODIACOS \
		-o $(ISO_IMG) \
		$(ISO_ROOT)
	@echo ""
	@echo "       ISO created successfully!"
	@echo "       Version:      $(VERSION)"
	@echo "       Architecture: $(ARCH)"
	@echo "       Output:       $(ISO_IMG)"
	@echo ""

iso-inspect: iso
	@echo "[ISO]  Inspecting ZodiacOS ISO..."
	xorriso -indev $(ISO_IMG) -report_el_torito as_mkisofs

run: $(DISK_IMG) $(DATA_IMG)
	$(QEMU) $(QEMU_DRIVES)

# Stops at the first fault and logs CPU resets/interrupts instead of rebooting
run-debug: $(DISK_IMG) $(DATA_IMG)
	$(QEMU) $(QEMU_DRIVES) -d int,cpu_reset -no-reboot -D $(BUILD_DIR)/qemu.log
	@echo "Log written to $(BUILD_DIR)/qemu.log"

run-hvf:
	@echo "run-hvf is only for ARCH=aarch64 (x86 guests can't use HVF on Apple Silicon)"

endif

# =============================================================================
#  aarch64: ELF loaded directly by QEMU's "virt" machine (-kernel)
# =============================================================================
ifeq ($(ARCH),aarch64)

AA64_DIR   := bootloader/aarch64
AA64_LD    := $(AA64_DIR)/linker.ld
AA64_OBJS  := $(BUILD_DIR)/entry.o $(BUILD_DIR)/exception_vectors.o
KERNEL_ELF := $(BUILD_DIR)/kernel-aarch64.elf

QEMU_MACHINE := -M virt -m 256M -nographic

all: $(KERNEL_ELF)
	@echo ""
	@echo "  Build complete! $(KERNEL_ELF)"
	@echo "  Run with:  make ARCH=aarch64 run       (software emulation)"
	@echo "         or: make ARCH=aarch64 run-hvf   (native speed on Apple Silicon)"
	@echo "  Leave QEMU with Ctrl-A, then X"
	@echo ""

$(BUILD_DIR)/%.o: $(AA64_DIR)/%.S | dirs
	@echo "[AS]   $<"
	$(CLANG) --target=aarch64-none-elf -c $< -o $@

$(KERNEL_ELF): $(AA64_OBJS) $(KERNEL_LIB) $(AA64_LD) | dirs
	@echo "[LD]   Linking AArch64 kernel..."
	$(LLD) -T $(AA64_LD) -o $@ $(AA64_OBJS) $(KERNEL_LIB)

run: $(KERNEL_ELF)
	$(QEMU) $(QEMU_MACHINE) -cpu cortex-a72 -kernel $(KERNEL_ELF)

# Runs the guest natively through Apple's Hypervisor framework
run-hvf: $(KERNEL_ELF)
	$(QEMU) $(QEMU_MACHINE) -accel hvf -cpu host -kernel $(KERNEL_ELF)

run-debug: $(KERNEL_ELF)
	$(QEMU) $(QEMU_MACHINE) -cpu cortex-a72 -d int,guest_errors -no-reboot -D $(BUILD_DIR)/qemu.log -kernel $(KERNEL_ELF)
	@echo "Log written to $(BUILD_DIR)/qemu.log"

endif

# ---- Watch mode: rebuild and relaunch QEMU on every save ----------------------
# Requires: brew install watchexec
watch:
	watchexec -r -w kernel/src -w kernel/Cargo.toml -w kernel/linker.ld -w bootloader -- $(MAKE) ARCH=$(ARCH) run

# ---- Clean ------------------------------------------------------------------
clean:
	@echo "[CLN]  Cleaning..."
	rm -rf $(BUILD_DIR)
	cd kernel && cargo clean
	@echo "       Done. (data disk in disk/ was kept)"

# Wipes the x86 data disk too (all files you saved inside ZodiacOS)
clean-disk:
	rm -rf disk