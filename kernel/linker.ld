ENTRY(_start)

SECTIONS
{
    . = 0x100000;
    __kernel_start = .;

    /* Code: read + execute. _start stays first so the entry point is at 0x100000. */
    . = ALIGN(4096);
    __text_start = .;
    .text   : { *(.text._start) *(.text .text.*) }
    . = ALIGN(4096);
    __text_end = .;

    /* Read-only data: read, no execute. */
    __rodata_start = .;
    .rodata : { *(.rodata .rodata.*) *(.data.rel.ro .data.rel.ro.*) }
    . = ALIGN(4096);
    __rodata_end = .;

    /* Writable data and bss: read + write, no execute. */
    __data_start = .;
    .data   : { *(.data .data.*) *(.got .got.*) }
    .bss    : {
        __bss_start = .;
        *(COMMON) *(.bss .bss.*)
        . = ALIGN(8);
        __bss_end = .;
    }
    . = ALIGN(4096);
    __kernel_end = .;

    /DISCARD/ : { *(.eh_frame*) *(.comment) }
}