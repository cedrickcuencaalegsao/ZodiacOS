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

# ---- VM size ------------------------------------------------------------------
# RAM given to the guest, and the size of the persistent data disk.
# Override by hand with:  make run MEM=4G DATA_SIZE=32G
# The data disk is a sparse qcow2 file: it starts tiny and only grows as the
# guest writes to it, so 24G does not use 24G of your Mac's storage.
# NOTE: DATA_SIZE only applies when the disk is created. To resize an existing
# disk run:  qemu-img resize disk/data.qcow2 32G   (or make clean-disk)
MEM       ?= 4G
CPUS	  ?= 4
DATA_SIZE ?= 24G

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

# One ISO per architecture, each packed from its own staging folder
ISO_IMG    := $(BUILD_DIR)/zodiacos-v$(VERSION)-$(ARCH).iso
ISO_ROOT   := $(BUILD_DIR)/iso-$(ARCH)

.DEFAULT_GOAL := all
.PHONY: all clean clean-disk run run-hvf run-debug watch dirs net-info \
        iso iso-x86_64 iso-aarch64 iso-all iso-inspect _iso

dirs:
	@mkdir -p $(BUILD_DIR)

# ---- Kernel (Rust), shared by every architecture ------------------------------
$(KERNEL_LIB): $(KERNEL_SRC) kernel/Cargo.toml
	echo "[RUST] Building kernel (release, $(TARGET))..."
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

# Data disk lives outside build/ so `make clean` never deletes your files.
# qcow2 = QEMU's own sparse image format. The guest still sees a plain 24G IDE
# disk (raw sectors); QEMU does the translation, and every write the guest
# makes is saved in this file, so data survives reboots and rebuilds.
DATA_DIR   := disk
DATA_IMG   := $(DATA_DIR)/data.qcow2

# ---- Kernel size, handed to the bootloader automatically ----------------------
# The BIOS loads the kernel in CHUNK_SECTORS-sector reads into low memory at
# 0x10000. That area ends where the BIOS data area starts (0x9FC00), which caps
# the image at 1150 sectors; rounded down to whole chunks it is 1144 sectors.
# boot.bin is assembled AFTER the kernel with -DKERNEL_SECTORS=<its real size>,
# so nothing has to be edited by hand when the kernel grows.
CHUNK_SECTORS      := 8
MAX_KERNEL_SECTORS := 1144
MAX_KERNEL_BYTES   := 585728

# ---- Host screen size ---------------------------------------------------------
# Asks macOS for the main screen size (in points). The size is handed to QEMU's
# VGA card as its preferred mode (EDID); the kernel reads it back at boot.
# Override by hand with:  make run SCREEN_SIZE=1920x1080
# Falls back to 1920x1080 if the query fails.
SCREEN_SIZE ?= $(shell osascript -l JavaScript -e 'ObjC.import("AppKit"); var s=$$.NSScreen.mainScreen.frame.size; Math.round(s.width)+"x"+Math.round(s.height)' 2>/dev/null)
ifeq ($(SCREEN_SIZE),)
  SCREEN_SIZE := 1920x1080
endif
SCREEN_W := $(word 1,$(subst x, ,$(SCREEN_SIZE)))
SCREEN_H := $(word 2,$(subst x, ,$(SCREEN_SIZE)))

# ---- Network (QEMU user-mode / SLIRP) -----------------------------------------
# QEMU runs its own virtual LAN and NATs it out through your Mac's real
# connection (WiFi or the USB ethernet dongle, it does not matter which).
# The guest never sees the dongle. ZodiacOS only sees this virtual e1000 card.
#
# IMPORTANT: this Makefile only controls what QEMU offers. The kernel itself
# must put these same values on eth0 (static config) or ask for them with a
# DHCP client. Until it does, pings fail with "no route to host".
#
#   Guest IP    10.0.2.15   (also the first address QEMU's DHCP server hands out)
#   Netmask     255.255.255.0
#   Gateway     10.0.2.2    (QEMU answers ARP and ping for this itself)
#   DNS         10.0.2.3
#
# Change the card with:  make run NIC=virtio-net-pci
NIC          ?= e1000
NET_SUBNET   := 10.0.2.0/24
NET_GATEWAY  := 10.0.2.2
NET_DNS      := 10.0.2.3
NET_GUEST_IP := 10.0.2.15
NET_PCAP     := $(BUILD_DIR)/net.pcap

# One NIC only. Two -nic user,... lines create two separate networks that
# both use 10.0.2.x, which is confusing when debugging.
# filter-dump records every frame the guest sends/receives; open it in Wireshark.
QEMU_NET := \
	-netdev user,id=net0,net=$(NET_SUBNET),host=$(NET_GATEWAY),dns=$(NET_DNS),dhcpstart=$(NET_GUEST_IP) \
	-device $(NIC),netdev=net0 \
	-object filter-dump,id=dump0,netdev=net0,file=$(NET_PCAP)

# COM1 output (slog!/log_info! in the kernel) goes to this terminal.
# To keep the terminal quiet and read logs later, use instead:
#   -serial file:$(BUILD_DIR)/serial.log
QEMU_DRIVES := \
	-m $(MEM) \
	-smp $(CPUS) \
	$(QEMU_NET) \
	-vga none \
	-device VGA,edid=on,xres=$(SCREEN_W),yres=$(SCREEN_H),vgamem_mb=32 \
	-display cocoa \
	-rtc base=localtime \
	-serial stdio \
	-drive format=raw,file=$(DISK_IMG),if=ide,index=0 \
	-drive format=qcow2,file=$(DATA_IMG),if=ide,index=1
# To see a USB controller in `lspci`, add:  -device qemu-xhci -device usb-kbd

all: $(DISK_IMG) $(DATA_IMG)
	echo ""
	echo "  Build complete! Disk image: $(DISK_IMG)"
	echo "  Run with:  make run     (screen: $(SCREEN_W)x$(SCREEN_H), RAM: $(MEM), data disk: $(DATA_SIZE))"
	echo ""

$(KERNEL_ELF): $(KERNEL_LIB) $(LINKER) | dirs
	echo "[LD]   Linking kernel ELF..."
	$(LLD) -u _start -T $(LINKER) -o $@ $(KERNEL_LIB)

$(KERNEL_BIN): $(KERNEL_ELF)
	echo "[OBJ]  Extracting flat binary..."
	$(OBJCOPY) -O binary $< $@
	echo "       $$(wc -c < $@) bytes (flat binary)"
	test $$(wc -c < $@) -le $(MAX_KERNEL_BYTES) || \
		(echo "ERROR: kernel is bigger than the $(MAX_KERNEL_BYTES)-byte real-mode load limit (0x9FC00)." && \
		 echo "       Shrink it (opt-level, lto, strip, big static arrays) or load it above 1 MiB." && exit 1)

# The bootloader is built after the kernel: it needs the kernel's size.
$(BOOT_BIN): $(BOOT_ASM) $(BOOT_INCLUDES) $(KERNEL_BIN) | dirs
	@sectors=$$(( ( $$(wc -c < $(KERNEL_BIN)) + 511 ) / 512 )); \
	sectors=$$(( ( sectors + $(CHUNK_SECTORS) - 1 ) / $(CHUNK_SECTORS) * $(CHUNK_SECTORS) )); \
	echo "[ASM]  Assembling Bootloader ($(ARCH)), loading $$sectors of $(MAX_KERNEL_SECTORS) sectors..."; \
	nasm -f bin -I bootloader/$(ARCH)/ -DKERNEL_SECTORS=$$sectors -DCHUNK=$(CHUNK_SECTORS) -o $@ $<
	@test $$(wc -c < $@) -eq 512 || (echo "ERROR: Bootloader is not 512 bytes!" && exit 1)
	echo "       $$(wc -c < $@) bytes — OK"
	@printf "       Boot signature: "; xxd -s 510 -l 2 $@

# ---- Initramfs (becomes the root ramfs, including /boot) ----------------------
# The bootloader and kernel are written to the disk as raw sectors, so they are
# not files. To make /boot show something real, copies of boot.bin and
# kernel.bin are put into initramfs/boot/ before the image is packed.
# (Add initramfs/boot/ to .gitignore, it is regenerated on every build.)
INITRAMFS_DIR := initramfs
INITRAMFS_IMG := $(BUILD_DIR)/initramfs.img
INITRAMFS_LBA := 2048

$(INITRAMFS_IMG): $(BOOT_BIN) $(KERNEL_BIN) $(shell find $(INITRAMFS_DIR) -type f 2>/dev/null) scripts/mkinitramfs.py | dirs
	echo "[INITRD] Packing $(INITRAMFS_DIR)/ ..."
	mkdir -p $(INITRAMFS_DIR)/boot
	cp $(BOOT_BIN)   $(INITRAMFS_DIR)/boot/boot.bin
	cp $(KERNEL_BIN) $(INITRAMFS_DIR)/boot/kernel.bin
	python3 scripts/mkinitramfs.py $(INITRAMFS_DIR) $@

# NOTE: this boot disk is recreated from zeros on every build, so never keep
# user files on it. Persistent files belong on the data disk below.
$(DISK_IMG): $(BOOT_BIN) $(KERNEL_BIN) $(INITRAMFS_IMG)
	echo "[IMG]  Creating disk image..."
	dd if=/dev/zero        of=$@ bs=1m  count=64 status=none
	dd if=$(BOOT_BIN)      of=$@ bs=512 seek=0 conv=notrunc status=none
	dd if=$(KERNEL_BIN)    of=$@ bs=512 seek=1 conv=notrunc status=none
	dd if=$(INITRAMFS_IMG) of=$@ bs=512 seek=$(INITRAMFS_LBA) conv=notrunc status=none
	echo "       Disk image ready: $@"

# Data disk (created once, never rebuilt). Sparse qcow2, see DATA_IMG above.
$(DATA_IMG):
	echo "[DATA] Creating $(DATA_SIZE) data disk (sparse qcow2)..."
	@mkdir -p $(DATA_DIR)
	qemu-img create -f qcow2 $@ $(DATA_SIZE)

# NOTE: the ISO holds the disk image as a file. It has no El Torito boot
# record yet, so boot build/zodiacos.img itself (make run).
_iso: $(DISK_IMG)
	echo "[ISO]  Creating ZodiacOS v$(VERSION) ISO for $(ARCH)..."
	@rm -rf $(ISO_ROOT)
	@mkdir -p $(ISO_ROOT)
	cp $(DISK_IMG) $(ISO_ROOT)/zodiacos.img
	xorriso -as mkisofs \
		-V ZODIACOS \
		-o $(ISO_IMG) \
		$(ISO_ROOT)
	echo ""
	echo "       ISO created successfully!"
	echo "       Version:      $(VERSION)"
	echo "       Architecture: $(ARCH)"
	echo "       Output:       $(ISO_IMG)"
	echo ""

run: $(DISK_IMG) $(DATA_IMG)
	echo "[QEMU] Requesting $(SCREEN_W)x$(SCREEN_H) display, $(MEM) RAM"
	$(QEMU) $(QEMU_DRIVES)

# Stops at the first fault and logs CPU resets/interrupts instead of rebooting
run-debug: $(DISK_IMG) $(DATA_IMG)
	$(QEMU) $(QEMU_DRIVES) -d int,cpu_reset -no-reboot -D $(BUILD_DIR)/qemu.log
	echo "Log written to $(BUILD_DIR)/qemu.log"

# Prints the network values the kernel must use, and where the capture goes
net-info:
	echo ""
	echo "  NIC model:   $(NIC)"
	echo "  Guest IP:    $(NET_GUEST_IP)"
	echo "  Netmask:     255.255.255.0  ($(NET_SUBNET))"
	echo "  Gateway:     $(NET_GATEWAY)"
	echo "  DNS:         $(NET_DNS)"
	echo "  Capture:     $(NET_PCAP)  (open in Wireshark)"
	echo ""
	echo "  Test order inside ZodiacOS:  ping $(NET_GATEWAY)  then  ping 8.8.8.8"
	echo ""

run-hvf:
	echo "run-hvf is only for ARCH=aarch64 (x86 guests can't use HVF on Apple Silicon)"

endif

# =============================================================================
#  aarch64: ELF loaded directly by QEMU's "virt" machine (-kernel)
# =============================================================================
ifeq ($(ARCH),aarch64)

AA64_DIR   := bootloader/aarch64
AA64_LD    := $(AA64_DIR)/linker.ld
AA64_OBJS  := $(BUILD_DIR)/entry.o $(BUILD_DIR)/exception_vectors.o
KERNEL_ELF := $(BUILD_DIR)/kernel-aarch64.elf

QEMU_MACHINE := -M virt -m $(MEM) -smp $(CPUS) -nographic

all: $(KERNEL_ELF)
	echo ""
	echo "  Build complete! $(KERNEL_ELF)"
	echo "  Run with:  make ARCH=aarch64 run       (software emulation)"
	echo "         or: make ARCH=aarch64 run-hvf   (native speed on Apple Silicon)"
	echo "  Leave QEMU with Ctrl-A, then X"
	echo ""

$(BUILD_DIR)/%.o: $(AA64_DIR)/%.S | dirs
	echo "[AS]   $<"
	$(CLANG) --target=aarch64-none-elf -c $< -o $@

$(KERNEL_ELF): $(AA64_OBJS) $(KERNEL_LIB) $(AA64_LD) | dirs
	echo "[LD]   Linking AArch64 kernel..."
	$(LLD) -T $(AA64_LD) -o $@ $(AA64_OBJS) $(KERNEL_LIB)

# NOTE: the ISO is a container for the kernel ELF. It is not UEFI-bootable yet
# (that needs the BOOTAA64.EFI stub), so run the ELF with QEMU instead.
_iso: $(KERNEL_ELF)
	echo "[ISO]  Creating ZodiacOS v$(VERSION) ISO for $(ARCH)..."
	@rm -rf $(ISO_ROOT)
	@mkdir -p $(ISO_ROOT)
	cp $(KERNEL_ELF) $(ISO_ROOT)/kernel-aarch64.elf
	@printf '%s\n' \
		"ZodiacOS v$(VERSION) for AArch64" \
		"" \
		"Run the kernel in QEMU:" \
		"  qemu-system-aarch64 -M virt -m 256M -nographic -cpu cortex-a72 -kernel kernel-aarch64.elf" \
		"" \
		"This ISO is a container for the kernel. It is not UEFI-bootable yet, see README.txt inside" \
		> $(ISO_ROOT)/README.txt
	xorriso -as mkisofs \
		-V ZODIACOS \
		-o $(ISO_IMG) \
		$(ISO_ROOT)
	echo ""
	echo "       ISO created successfully!"
	echo "       Version:      $(VERSION)"
	echo "       Architecture: $(ARCH)"
	echo "       Output:       $(ISO_IMG)"
	echo "       Note:         not UEFI-bootable yet, see README.txt inside"
	echo ""

run: $(KERNEL_ELF)
	$(QEMU) $(QEMU_MACHINE) -cpu cortex-a72 -kernel $(KERNEL_ELF)

# Runs the guest natively through Apple's Hypervisor framework
run-hvf: $(KERNEL_ELF)
	$(QEMU) $(QEMU_MACHINE) -accel hvf -cpu host -kernel $(KERNEL_ELF)

run-debug: $(KERNEL_ELF)
	$(QEMU) $(QEMU_MACHINE) -cpu cortex-a72 -d int,guest_errors -no-reboot -D $(BUILD_DIR)/qemu.log -kernel $(KERNEL_ELF)
	echo "Log written to $(BUILD_DIR)/qemu.log"

net-info:
	echo "net-info is only for ARCH=x86_64 (the aarch64 virt target has no NIC configured yet)"

endif

# ---- ISO commands ---------------------------------------------------------------
# Each one re-runs make with the right ARCH, so it works from any setting.
iso-x86_64:
	@$(MAKE) --no-print-directory ARCH=x86_64 _iso

iso-aarch64:
	@$(MAKE) --no-print-directory ARCH=aarch64 _iso

iso-all: iso-x86_64 iso-aarch64

# `make iso` builds the ISO for the current ARCH (default x86_64)
iso: iso-$(ARCH)

iso-inspect: iso
	echo "[ISO]  Inspecting $(ISO_IMG)..."
	xorriso -indev $(ISO_IMG) -report_el_torito as_mkisofs

# ---- Watch mode: rebuild and relaunch QEMU on every save ----------------------
# Requires: brew install watchexec
watch:
	watchexec -r -w kernel/src -w kernel/Cargo.toml -w kernel/linker.ld -w bootloader -- $(MAKE) ARCH=$(ARCH) run

# ---- Clean ------------------------------------------------------------------
clean:
	echo "[CLN]  Cleaning..."
	rm -rf $(BUILD_DIR)
	cd kernel && cargo clean && cd ..
	cd userspace/desktop && cargo clean && cd ../../
	cd abi && cargo clean && cd ..
	cd zpm && cargo clean && cd ..
	echo "       Done. (data disk in disk/ was kept)"

# Wipes the x86 data disk too (all files you saved inside ZodiacOS)
clean-disk:
	rm -rf disk