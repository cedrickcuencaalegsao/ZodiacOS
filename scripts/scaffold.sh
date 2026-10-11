#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

dirs=(
  .vscode bootloader/x86_64 bootloader/aarch64
  zpm-repo/config zpm-repo/manifests/hello zpm-repo/packages/x86_64 zpm-repo/packages/aarch64
  zpm-repo/keys zpm-repo/tools
  packages/hello packages/shell packages/coreutils packages/init
  userspace/init/src userspace/shell userspace/coreutils userspace/libzodiac
  targets sysroot images scripts
  tests/boot tests/filesystem tests/packages tests/integration
)

files=(
  bootloader/x86_64/{boot.asm,gdt.asm,long_mode.asm,linker.ld}
  bootloader/aarch64/{entry.S,exception_vectors.S,linker.ld}
  kernel/linker.ld
  kernel/src/arch/mod.rs
  kernel/src/arch/x86_64/{mod,cpu,gdt,interrupts,paging,port_io}.rs
  kernel/src/arch/aarch64/{mod,cpu,exceptions,paging,registers}.rs
  kernel/src/base/{mod,error,logging,sync}.rs
  kernel/src/memory/{mod,physical,virtual_memory,allocator,heap}.rs
  kernel/src/interrupts/{mod,timer,controller}.rs
  kernel/src/process/{mod,task,context,syscall}.rs
  kernel/src/scheduler/{mod,run_queue,policy}.rs
  kernel/src/filesystem/{mod,vfs,inode,path,mount,ramfs,devfs,procfs,zdfs}.rs
  kernel/src/drivers/{mod,serial,framebuffer,keyboard,rtc}.rs
  kernel/src/drivers/storage/{mod,ata,virtio_blk}.rs
  kernel/src/drivers/network/{mod,virtio_net,ethernet}.rs
  kernel/src/network/{mod,ethernet,ipv4,udp,tcp,dns,socket}.rs
  kernel/src/security/{mod,permissions,capabilities}.rs
  kernel/src/user/{mod,elf,loader}.rs
  kernel/src/console/{mod,terminal,splash,memcmd}.rs
  kernel/src/legacy/mod.rs
  zpm/src/{main,cli,config,package,manifest,repository,download,signature,dependency,database,install,remove,upgrade,verify,transaction}.rs
  zpm-repo/{README.md,config/repository.toml,manifests/hello/1.0.0.toml,tools/index.rs,tools/sign.rs}
  packages/hello/{manifest.toml,build.sh}
  userspace/init/src/main.rs
  targets/{x86_64-zodiac.json,aarch64-zodiac.json}
  sysroot/README.md images/README.md images/.gitkeep
  scripts/{build-kernel.sh,build-image.sh,run-qemu-x86_64.sh,run-qemu-aarch64.sh,check-targets.sh}
)

for d in "${dirs[@]}"; do mkdir -p "$d"; done

for f in "${files[@]}"; do
  mkdir -p "$(dirname "$f")"
  [ -e "$f" ] && continue
  case "$f" in
    *.rs)       printf '//! %s (not implemented yet)\n' "${f#kernel/src/}" > "$f" ;;
    *.asm)      printf '; %s (placeholder)\n' "$f" > "$f" ;;
    *.S)        printf '// %s (placeholder)\n' "$f" > "$f" ;;
    *.ld)       printf '/* %s (placeholder) */\n' "$f" > "$f" ;;
    *.toml)     printf '# %s (placeholder)\n' "$f" > "$f" ;;
    *.json)     printf '{}\n' > "$f" ;;
    *.md)       printf '# %s\n' "$(basename "$(dirname "$f")")" > "$f" ;;
    *.sh)       printf '#!/usr/bin/env bash\nset -euo pipefail\n# %s (placeholder)\n' "$f" > "$f"; chmod +x "$f" ;;
    *)          : > "$f" ;;
  esac
done

echo "Tree ready."