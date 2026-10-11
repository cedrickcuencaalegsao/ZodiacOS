ENTRY(_start)

SECTIONS
{
    . = 0x10000;
    __kernel_start = .;

    __text_start = .;
    .text   : { *(.text._start) *(.text .text.*) }
    . = ALIGN(4096);
    __text_end = .;

    __rodata_start = .;
    .rodata : { *(.rodata .rodata.*) }
    . = ALIGN(4096);
    __rodata_end = .;

    .data   : { *(.data .data.*) }
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