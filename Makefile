# ---- Config -----------------------------------------------------------------
TARGET     := x86_64-unknown-none
BUILD_DIR  := build
LLD        := /opt/homebrew/bin/ld.lld
OBJCOPY    := /opt/homebrew/opt/llvm/bin/llvm-objcopy

BOOT_BIN   := $(BUILD_DIR)/boot.bin
KERNEL_LIB := kernel/target/$(TARGET)/release/libzodiac_os.a
KERNEL_ELF := $(BUILD_DIR)/kernel.elf
KERNEL_BIN := $(BUILD_DIR)/kernel.bin
DISK_IMG   := $(BUILD_DIR)/zodiacos.img

ISO_IMG    := $(BUILD_DIR)/zodiacos.iso
ISO_ROOT   := $(BUILD_DIR)/iso

# Data disk lives outside build/ so `make clean` never deletes your files
DATA_DIR   := disk
DATA_IMG   := $(DATA_DIR)/data.img

# Must match KERNEL_SECTORS in bootloader/boot.asm (640 sectors * 512 bytes)
MAX_KERNEL_BYTES := 327680

KERNEL_SRC := $(shell find kernel/src -name '*.rs')

# COM1 output (slog!/log_info! in the kernel) goes to this terminal.
# To keep the terminal quiet and read logs later, use instead:
#   -serial file:$(BUILD_DIR)/serial.log
QEMU       := qemu-system-x86_64
QEMU_DRIVES := \
	-rtc base=localtime \
	-serial stdio \
	-drive format=raw,file=$(DISK_IMG),if=ide,index=0 \
	-drive format=raw,file=$(DATA_IMG),if=ide,index=1

.PHONY: all clean clean-disk run run-debug watch dirs iso iso-inspect

# ---- Default ----------------------------------------------------------------
all: $(DISK_IMG) $(DATA_IMG)
	@echo ""
	@echo "  Build complete! Disk image: $(DISK_IMG)"
	@echo "  Run with:  make run"
	@echo ""

dirs:
	@mkdir -p $(BUILD_DIR)

# ---- Bootloader -------------------------------------------------------------
$(BOOT_BIN): bootloader/boot.asm | dirs
	@echo "[ASM]  Assembling Bootloader..."
	nasm -f bin -o $@ $<
	@test $$(wc -c < $@) -eq 512 || (echo "ERROR: Bootloader is not 512 bytes!" && exit 1)
	@echo "       $$(wc -c < $@) bytes — OK"
	@printf "       Boot signature: "; xxd -s 510 -l 2 $@

# ---- Kernel (Rust) ----------------------------------------------------------
$(KERNEL_LIB): $(KERNEL_SRC) kernel/Cargo.toml
	@echo "[RUST] Building kernel (release)..."
	cd kernel && cargo +nightly build --release \
		-Z build-std=core,compiler_builtins \
		-Z build-std-features=compiler-builtins-mem \
		--target $(TARGET)

$(KERNEL_ELF): $(KERNEL_LIB) kernel/src/linker.ld | dirs
	@echo "[LD]   Linking kernel ELF..."
	$(LLD) -u _start -T kernel/src/linker.ld -o $@ $(KERNEL_LIB)

$(KERNEL_BIN): $(KERNEL_ELF)
	@echo "[OBJ]  Extracting flat binary..."
	$(OBJCOPY) -O binary $< $@
	@echo "       $$(wc -c < $@) bytes (flat binary)"
	@test $$(wc -c < $@) -le $(MAX_KERNEL_BYTES) || \
		(echo "ERROR: kernel exceeds $(MAX_KERNEL_BYTES) bytes. Raise KERNEL_SECTORS in boot.asm and MAX_KERNEL_BYTES here." && exit 1)

# ---- Boot disk --------------------------------------------------------------
$(DISK_IMG): $(BOOT_BIN) $(KERNEL_BIN)
	@echo "[IMG]  Creating disk image..."
	dd if=/dev/zero     of=$@ bs=1m  count=64 status=none
	dd if=$(BOOT_BIN)   of=$@ bs=512 seek=0 conv=notrunc status=none
	dd if=$(KERNEL_BIN) of=$@ bs=512 seek=1 conv=notrunc status=none
	@echo "       Disk image ready: $@"

# ---- Data disk (created once, never rebuilt) ---------------------------------
$(DATA_IMG):
	@echo "[DATA] Creating 16 MB data disk..."
	@mkdir -p $(DATA_DIR)
	dd if=/dev/zero of=$@ bs=1m count=16 status=none

# ---- ISO --------------------------------------------------------------------
# NOTE: this ISO is not bootable yet (no El Torito boot record), see the notes.
iso: $(DISK_IMG)
	@echo "[ISO]  Creating ZodiacOS ISO..."
	@mkdir -p $(ISO_ROOT)
	cp $(DISK_IMG) $(ISO_ROOT)/zodiacos.img
	xorriso -as mkisofs \
		-V ZODIACOS \
		-o $(ISO_IMG) \
		$(ISO_ROOT)
	@echo "       ISO created: $(ISO_IMG)"

iso-inspect: iso
	@echo "[ISO]  Inspecting ZodiacOS ISO..."
	xorriso -indev $(ISO_IMG) -report_el_torito as_mkisofs

# ---- QEMU -------------------------------------------------------------------
run: $(DISK_IMG) $(DATA_IMG)
	$(QEMU) $(QEMU_DRIVES)

# Stops at the first fault and logs CPU resets/interrupts instead of rebooting
run-debug: $(DISK_IMG) $(DATA_IMG)
	$(QEMU) $(QEMU_DRIVES) -d int,cpu_reset -no-reboot -D $(BUILD_DIR)/qemu.log
	@echo "Log written to $(BUILD_DIR)/qemu.log"

# ---- Watch mode: rebuild and relaunch QEMU on every save ----------------------
# Requires: brew install watchexec
watch:
	watchexec -r -w kernel/src -w kernel/Cargo.toml -w bootloader -- make run

# ---- Clean ------------------------------------------------------------------
clean:
	@echo "[CLN]  Cleaning..."
	rm -rf $(BUILD_DIR)
	cd kernel && cargo clean
	@echo "       Done. (data disk in $(DATA_DIR)/ was kept)"

# Wipes the data disk too (all files you saved inside ZodiacOS)
clean-disk:
	rm -rf $(DATA_DIR)