ENTRY(_start)

SECTIONS
{
    . = 0x10000;

    .text   : { *(.text._start) *(.text .text.*) }
    .rodata : { *(.rodata .rodata.*) }
    .data   : { *(.data .data.*) }
    .bss    : {
        __bss_start = .;
        *(COMMON) *(.bss .bss.*)
        . = ALIGN(8);
        __bss_end = .;
    }

    /DISCARD/ : { *(.eh_frame*) *(.comment) }
}