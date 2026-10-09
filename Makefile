TARGET     := x86_64-unknown-none
BUILD_DIR  := build
LLD        := /opt/homebrew/bin/ld.lld
OBJCOPY    := /opt/homebrew/opt/llvm/bin/llvm-objcopy

BOOT_BIN   := $(BUILD_DIR)/boot.bin
KERNEL_LIB := kernel/target/$(TARGET)/release/libzodiac_os.a
KERNEL_ELF := $(BUILD_DIR)/kernel.elf
KERNEL_BIN := $(BUILD_DIR)/kernel.bin
DISK_IMG   := $(BUILD_DIR)/zodiacos.img

# .PHONY: all clean run dirs
.PHONY: watch
watch:
	watchexec -r -w kernel/src -w kernel/Cargo.toml -w bootloader -- make run

all: $(DISK_IMG)
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

# ---- Kernel (Rust) ----------------------------------------------------------
$(KERNEL_LIB): $(wildcard kernel/src/*.rs) kernel/Cargo.toml
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

# ---- Disk Image -------------------------------------------------------------
$(DISK_IMG): $(BOOT_BIN) $(KERNEL_BIN)
	@echo "[IMG]  Creating disk image..."
	dd if=/dev/zero     of=$@ bs=1m  count=64 status=none
	dd if=$(BOOT_BIN)   of=$@ bs=512 seek=0 conv=notrunc status=none
	dd if=$(KERNEL_BIN) of=$@ bs=512 seek=1 conv=notrunc status=none
	@echo "       Disk image ready: $@"

# ---- QEMU -------------------------------------------------------------------
run: $(DISK_IMG)
	qemu-system-x86_64 -drive format=raw,file=$(DISK_IMG)

# ---- Clean ------------------------------------------------------------------
clean:
	@echo "[CLN]  Cleaning..."
	rm -rf $(BUILD_DIR)
	cd kernel && cargo clean