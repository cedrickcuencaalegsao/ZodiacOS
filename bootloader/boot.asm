; ZodiacOS bootloader: 16-bit real mode -> 64-bit long mode
; Assemble: nasm -f bin -o build/boot.bin bootloader/boot.asm

[bits 16]
[org 0x7C00]

KERNEL_ADDR     equ 0x10000     ; physical load address (linker script must match)
KERNEL_SEG      equ 0x1000      ; KERNEL_ADDR >> 4
KERNEL_SECTORS  equ 64          ; 32 KB; raise if your kernel grows
STACK_TOP       equ 0x90000

PML4            equ 0x1000
PDPT            equ 0x2000
PD              equ 0x3000

start:
    cli
    xor ax, ax
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov sp, 0x7C00
    sti

    mov [boot_drive], dl        ; BIOS passes boot drive in DL

    ; ---- Load kernel with BIOS extended read (LBA) ----
    mov si, dap
    mov dl, [boot_drive]
    mov ah, 0x42
    int 0x13
    jc  disk_error

    cli

    ; ---- Enable A20 (fast method) ----
    in   al, 0x92
    or   al, 2
    and  al, 0xFE               ; never set bit 0 (would reset the CPU)
    out  0x92, al

    ; ---- Zero the page table area (0x1000-0x3FFF) ----
    cld
    mov  edi, PML4
    xor  eax, eax
    mov  ecx, 3072              ; 12 KB / 4
    rep  stosd

    ; ---- Identity-map the first 2 MB using one 2 MB page ----
    mov  dword [PML4], PDPT | 0x03      ; present | writable
    mov  dword [PDPT], PD   | 0x03
    mov  dword [PD],   0x83             ; present | writable | page size (2MB)

    ; ---- Enter long mode ----
    lgdt [gdt_ptr]

    mov  eax, cr4
    or   eax, 1 << 5            ; PAE
    mov  cr4, eax

    mov  eax, PML4
    mov  cr3, eax

    mov  ecx, 0xC0000080        ; EFER MSR
    rdmsr
    or   eax, 1 << 8            ; LME
    wrmsr

    mov  eax, cr0
    or   eax, 0x80000001        ; PG | PE
    mov  cr0, eax

    jmp  0x08:long_mode

disk_error:
    mov  ah, 0x0E
    mov  al, 'E'
    int  0x10
.hang:
    hlt
    jmp  .hang

[bits 64]
long_mode:
    mov  ax, 0x10
    mov  ds, ax
    mov  es, ax
    mov  ss, ax
    xor  ax, ax
    mov  fs, ax
    mov  gs, ax

    mov  esp, STACK_TOP
    mov  eax, KERNEL_ADDR
    jmp  rax                    ; -> _start

; ---- Data ----
align 8
gdt:
    dq 0x0000000000000000       ; null
    dq 0x00209A0000000000       ; 0x08: 64-bit code (L=1)
    dq 0x0000920000000000       ; 0x10: data
gdt_end:

gdt_ptr:
    dw gdt_end - gdt - 1
    dd gdt

dap:                            ; Disk Address Packet
    db 0x10, 0
    dw KERNEL_SECTORS
    dw 0x0000, KERNEL_SEG       ; offset, segment
    dq 1                        ; start at LBA 1 (sector right after boot sector)

boot_drive: db 0

times 510 - ($ - $$) db 0
dw 0xAA55