#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

# bootloader
mkdir -p bootloader/x86_64
git mv bootloader/boot.asm bootloader/x86_64/boot.asm

cd kernel/src
mv_() { mkdir -p "$(dirname "$2")"; git mv "$1" "$2"; }

mv_ arch.rs            arch/x86_64/cpu.rs      # contents are replaced below
mv_ idt.rs             arch/x86_64/interrupts.rs
mv_ paging.rs          arch/x86_64/paging.rs
mv_ pmm.rs             memory/physical.rs
mv_ heap.rs            memory/heap.rs
mv_ sync.rs            base/sync.rs
mv_ interrupts.rs      interrupts/mod.rs
mv_ serial.rs          drivers/serial.rs
mv_ keyboard.rs        drivers/keyboard.rs
mv_ ata.rs             drivers/storage/ata.rs
mv_ time.rs            drivers/rtc.rs
mv_ user.rs            security/permissions.rs
mv_ fs.rs              filesystem/zdfs.rs
mv_ base/shell.rs      console/terminal.rs
mv_ base/splash.rs     console/splash.rs
mv_ base/memcmd.rs     console/memcmd.rs
mv_ base/zpm.rs        legacy/zpm.rs
mkdir -p legacy && git mv package legacy/package
mv_ linker.ld          ../linker.ld

echo "Moved. Now paste the new mod.rs / lib.rs files, then run scripts/scaffold.sh"