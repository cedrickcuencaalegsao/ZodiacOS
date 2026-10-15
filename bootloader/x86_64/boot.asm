; ZodiacOS x86_64 bootloader: one 512-byte sector.
; Real mode: read the BIOS memory map, load the kernel, enable A20,
; then long_mode.asm switches to 64-bit, copies the kernel to 1 MiB and jumps to it.
;
; Files:    boot.asm (this one), gdt.asm, long_mode.asm
; Assemble: the Makefile does it and passes the kernel size in, so nothing here
;           has to be edited when the kernel grows:
;             nasm -f bin -I bootloader/x86_64/ -DKERNEL_SECTORS=<n> -DCHUNK=<n> -o build/boot.bin bootloader/x86_64/boot.asm

[bits 16]
[org 0x7C00]

LOAD_ADDR       equ 0x10000     ; the BIOS can only read below 1 MiB, so the image is loaded here...
LOAD_SEG        equ 0x1000      ; LOAD_ADDR >> 4
KERNEL_ADDR     equ 0x100000    ; ...and copied here (the link address in kernel/linker.ld)
LOAD_LIMIT      equ 0x9FC00     ; start of the BIOS data area: the image must end below this

; Both come from the Makefile (-D). The defaults only apply if you assemble by hand.
%ifndef CHUNK
CHUNK           equ 8           ; sectors per BIOS read (4 KB)
%endif
%ifndef KERNEL_SECTORS
KERNEL_SECTORS  equ 1144        ; image size in sectors, a multiple of CHUNK
%endif

%if KERNEL_SECTORS % CHUNK != 0
%error KERNEL_SECTORS must be a multiple of CHUNK
%endif
%if KERNEL_SECTORS / CHUNK > 255
%error too many chunks for the 8-bit counter, raise CHUNK
%endif
%if LOAD_ADDR + KERNEL_SECTORS * 512 > LOAD_LIMIT
%error the kernel image does not fit below the BIOS data area at 0x9FC00
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
    mov es, ax
    mov ss, ax
    mov sp, 0x7C00
    sti

    mov [boot_drive], dl        ; BIOS passes the boot drive in DL

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

    ; ---- Load the kernel in chunks with BIOS extended read (LBA) ----
.load:
    mov  si, dap
    mov  dl, [boot_drive]
    mov  ah, 0x42
    int  0x13
    jc   disk_error
    add  word  [dap_seg], CHUNK * 512 / 16      ; advance destination
    add  dword [dap_lba], CHUNK                 ; advance source sector
    dec  byte  [chunks_left]
    jnz  .load

    ; ---- Enable A20 (fast method), needed to touch memory above 1 MiB ----
    cli
    in   al, 0x92
    or   al, 2
    and  al, 0xFE               ; never set bit 0 (it resets the CPU)
    out  0x92, al

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
dap:                            ; Disk Address Packet (updated after each chunk)
    db 0x10, 0
    dw CHUNK                    ; sectors per read
    dw 0x0000                   ; destination offset
dap_seg:
    dw LOAD_SEG                 ; destination segment
dap_lba:
    dq 1                        ; start at LBA 1

chunks_left: db KERNEL_SECTORS / CHUNK
boot_drive:  db 0

times 510 - ($ - $$) db 0
dw 0xAA55