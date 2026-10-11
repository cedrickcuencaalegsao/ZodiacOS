; gdt.asm - Global Descriptor Table for the real mode -> long mode switch.
; Included by boot.asm. Provides: gdt, gdt_ptr, CODE_SEL, DATA_SEL.
; This table only has to live until the kernel installs its own GDT.

align 8
gdt:
    dq 0x0000000000000000       ; 0x00  null descriptor
gdt_code:
    dq 0x00209A0000000000       ; 0x08  64-bit code: present, DPL 0, exec/read, L=1
gdt_data:
    dq 0x0000920000000000       ; 0x10  data: present, DPL 0, read/write
gdt_end:

gdt_ptr:
    dw gdt_end - gdt - 1        ; limit
    dd gdt                      ; base (32 bits is enough, we are still in real mode)

CODE_SEL equ gdt_code - gdt     ; 0x08
DATA_SEL equ gdt_data - gdt     ; 0x10