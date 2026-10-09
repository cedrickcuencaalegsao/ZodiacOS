ENTRY(_start)

SECTIONS
{
	. = 0x10000;

	.text	: { *(.text._start) *(.text .text.*) }
	.rodata	: { *(.rodata .rodata.*) }
	.data	: { *(.data .data.*) }
	.bss	: { *(COMMON) *(.bss .bss.*) }
	
	/DISCARD/ : { *(.eh_frame) *(.comment) }
}
