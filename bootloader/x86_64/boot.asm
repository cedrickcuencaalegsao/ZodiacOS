; ZodiacOS x86_64 bootloader: one 512-byte sector.
; Real mode: read the BIOS memory map, enable A20, then load the kernel in chunks:
; each chunk is read into a small low buffer (LOAD_ADDR) and immediately copied up to
; 1 MiB with BIOS INT 15h AH=87h. The kernel size is therefore no longer limited by
; the 0x9FC00 real-mode boundary. long_mode.asm then switches to 64-bit and jumps.
;
; Files:    boot.asm (this one), gdt.asm, long_mode.asm
; Assemble: the Makefile passes the kernel size in:
;             nasm -f bin -I bootloader/x86_64/ -DKERNEL_SECTORS=<n> -DCHUNK=64 -o build/boot.bin bootloader/x86_64/boot.asm

[bits 16]
[org 0x7C00]

LOAD_ADDR       equ 0x10000     ; low staging buffer, one CHUNK long (BIOS disk reads must be < 1 MiB)
LOAD_SEG        equ 0x1000      ; LOAD_ADDR >> 4
KERNEL_ADDR     equ 0x100000    ; final home of the kernel (the link address in kernel/linker.ld)

; Both come from the Makefile (-D). The defaults only apply if you assemble by hand.
%ifndef CHUNK
CHUNK           equ 64          ; sectors per BIOS read + copy (32 KB)
%endif
%ifndef KERNEL_SECTORS
KERNEL_SECTORS  equ 1408        ; image size in sectors, a multiple of CHUNK
%endif

%if KERNEL_SECTORS % CHUNK != 0
%error KERNEL_SECTORS must be a multiple of CHUNK
%endif
%if KERNEL_SECTORS / CHUNK > 255
%error too many chunks for the 8-bit counter, raise CHUNK
%endif
%if CHUNK * 512 > 0xFFFE
%error CHUNK too large: one copy is at most 64 KB
%endif
%if KERNEL_ADDR + KERNEL_SECTORS * 512 > 0x1000000
%error kernel must end below 16 MiB (the BIOS copy uses 24-bit addresses)
%endif

STACK_TOP       equ 0x90000

MEMMAP_ADDR     equ 0x4000      ; BIOS memory map: u32 count, 24-byte entries from +16
MEMMAP_MAX      equ 32

    jmp short start             ; skip over the GDT data below

%include "gdt.asm"              ; gdt, gdt_ptr, CODE_SEL, DATA_SEL

start:
    cli
    xor ax, ax
    mov ds, ax
    mov es, ax                  ; ES must stay 0: INT 15h AH=87h reads ES:SI
    mov ss, ax
    mov sp, 0x7C00
    sti

    mov [boot_drive], dl        ; BIOS passes the boot drive in DL

    ; ---- Enable A20 (fast method), needed to touch memory above 1 MiB ----
    in   al, 0x92
    or   al, 2
    and  al, 0xFE               ; never set bit 0 (it resets the CPU)
    out  0x92, al

    ; ---- Ask the BIOS for the physical memory map (INT 0x15, EAX=0xE820) ----
    xor  ebx, ebx               ; continuation value, 0 = first call
    xor  bp, bp                 ; entry count
    mov  di, MEMMAP_ADDR + 16
.e820:
    mov  dword [di + 20], 1     ; ACPI extended-attributes word (bit 0 = valid)
    mov  eax, 0xE820
    mov  edx, 0x534D4150        ; 'SMAP'
    mov  ecx, 24
    int  0x15
    jc   .e820_done
    cmp  eax, 0x534D4150
    jne  .e820_done
    inc  bp
    add  di, 24
    cmp  bp, MEMMAP_MAX
    jae  .e820_done
    test ebx, ebx
    jnz  .e820
.e820_done:
    mov  [MEMMAP_ADDR], bp
    mov  word [MEMMAP_ADDR + 2], 0

    ; ---- Load the kernel chunk by chunk (BIOS extended read, LBA), retrying on error ----
.load:
    mov  cx, 3                  ; attempts per chunk
.retry:
    mov  si, dap
    mov  dl, [boot_drive]
    mov  ah, 0x42
    int  0x13
    jnc  .ok
    xor  ah, ah                 ; reset the disk system and try again
    mov  dl, [boot_drive]
    int  0x13
    dec  cx
    jnz  .retry
    jmp  disk_error
.ok:
    ; copy this chunk from the staging buffer up to its place at 1 MiB+
    mov  ah, 0x87
    mov  cx, CHUNK * 512 / 2    ; count is in WORDS
    mov  si, xgdt               ; ES:SI -> descriptor table (ES = 0)
    int  0x15
    jc   disk_error
    add  word [xdst + 2], CHUNK * 512   ; advance destination base (24-bit)
    adc  byte [xdst + 4], 0
    add  dword [dap_lba], CHUNK         ; advance source sector
    dec  byte  [chunks_left]
    jnz  .load

    ; ---- Make sure the CPU supports long mode, otherwise we would triple fault ----
    cli
    mov  eax, 0x80000000
    cpuid
    cmp  eax, 0x80000001
    jb   disk_error
    mov  eax, 0x80000001
    cpuid
    bt   edx, 29                ; LM bit
    jnc  disk_error

    jmp  enter_long_mode        ; defined in long_mode.asm

disk_error:
    mov  ah, 0x0E
    mov  al, 'E'
    int  0x10
.hang:
    hlt
    jmp  .hang

%include "long_mode.asm"        ; enter_long_mode, long_mode_entry

; ---- Data ----
dap:                            ; Disk Address Packet (only the LBA changes between reads)
    db 0x10, 0
    dw CHUNK                    ; sectors per read
    dw 0x0000                   ; destination offset
    dw LOAD_SEG                 ; destination segment (always the same staging buffer)
dap_lba:
    dq 1                        ; start at LBA 1

; Descriptor table for INT 15h AH=87h (6 descriptors, 8 bytes each)
xgdt:
    dq 0                                ; 0: dummy
    dq 0                                ; 1: filled in by the BIOS
    dw 0xFFFF, LOAD_ADDR & 0xFFFF       ; 2: source  (limit, base 15:0)
    db (LOAD_ADDR >> 16) & 0xFF, 0x93, 0, 0
xdst:
    dw 0xFFFF, KERNEL_ADDR & 0xFFFF     ; 3: destination (advanced after every chunk)
    db (KERNEL_ADDR >> 16) & 0xFF, 0x93, 0, 0
    dq 0                                ; 4: BIOS code segment (filled by the BIOS)
    dq 0                                ; 5: BIOS stack segment (filled by the BIOS)

chunks_left: db KERNEL_SECTORS / CHUNK
boot_drive:  db 0

times 510 - ($ - $$) db 0
dw 0xAA55