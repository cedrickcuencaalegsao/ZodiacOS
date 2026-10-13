; long_mode.asm - switch the CPU from 16-bit real mode to 64-bit long mode,
; copy the kernel to its final address, then jump to it.
; Included by boot.asm. Needs LOAD_ADDR, KERNEL_ADDR, KERNEL_SECTORS, STACK_TOP
; (boot.asm) and gdt_ptr, CODE_SEL, DATA_SEL (gdt.asm).

PML4            equ 0x1000
PDPT            equ 0x2000
PD              equ 0x3000

[bits 16]
enter_long_mode:
    ; ---- Zero the three page tables (0x1000-0x3FFF) ----
    cld
    mov  edi, PML4
    xor  eax, eax
    mov  ecx, 3072              ; 12 KB / 4
    rep  stosd

    mov  dword [PML4], PDPT | 0x03      ; present | writable
    mov  dword [PDPT], PD   | 0x03

    ; ---- Identity-map the first 1 GiB with 512 two-megabyte pages ----
    mov  edi, PD
    mov  eax, 0x83              ; present | writable | 2 MB page
    mov  ecx, 512
.fill_pd:
    mov  [edi], eax             ; (the upper dword was zeroed above)
    add  eax, 0x200000
    add  edi, 8
    loop .fill_pd

    ; ---- Load the GDT and enable long mode ----
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

    jmp  CODE_SEL:long_mode_entry       ; far jump loads the 64-bit code segment

[bits 64]
long_mode_entry:
    mov  ax, DATA_SEL
    mov  ds, ax
    mov  es, ax
    mov  ss, ax
    xor  ax, ax
    mov  fs, ax
    mov  gs, ax

    ; ---- Copy the kernel from the low load area to its home at 1 MiB ----
    cld
    mov  esi, LOAD_ADDR
    mov  edi, KERNEL_ADDR
    mov  ecx, KERNEL_SECTORS * 512 / 8
    rep  movsq

    mov  esp, STACK_TOP
    mov  eax, KERNEL_ADDR
    jmp  rax                    ; -> _start in the kernel