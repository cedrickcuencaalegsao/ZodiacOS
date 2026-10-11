/* AArch64 kernel link script (QEMU "virt": RAM starts at 0x40000000) */
ENTRY(_start)

SECTIONS
{
    . = 0x40080000;
    __kernel_start = .;

    __text_start = .;
    .text : {
        KEEP(*(.text.boot))         /* entry.S first */
        *(.text .text.*)            /* includes .text.vectors, aligned to 2 KiB */
    }
    . = ALIGN(4096);
    __text_end = .;

    __rodata_start = .;
    .rodata : { *(.rodata .rodata.*) }
    . = ALIGN(4096);
    __rodata_end = .;

    .data : { *(.data .data.*) }

    .bss (NOLOAD) : {
        . = ALIGN(16);
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        . = ALIGN(16);
        __bss_end = .;
    }

    . = ALIGN(4096);
    . += 0x10000;                   /* 64 KiB boot stack */
    __stack_top = .;
    __kernel_end = .;

    /DISCARD/ : { *(.comment) *(.eh_frame*) *(.note*) }
}