// AArch64 entry point. QEMU "virt" (-kernel) and most firmware hand over here
// with the MMU off and x0 = pointer to the device tree.
// Core 0 continues; every other core parks.

.section .text.boot, "ax"
.global _start
_start:
    mov     x19, x0                     // keep the device tree pointer (callee-saved)

    mrs     x1, mpidr_el1               // only core 0 runs the kernel
    and     x1, x1, #0xFF
    cbnz    x1, park

    mrs     x1, CurrentEL               // which exception level did we start in?
    lsr     x1, x1, #2
    cmp     x1, #2
    b.eq    from_el2
    cmp     x1, #1
    b.eq    el1_main
    b       park                        // EL3 / EL0: not supported

// Started at EL2 (hypervisor level): configure it, then drop to EL1.
from_el2:
    mov     x1, #0x80000000             // HCR_EL2.RW: EL1 runs AArch64
    msr     hcr_el2, x1
    mrs     x1, cnthctl_el2             // let EL1 use the timer and counter
    orr     x1, x1, #3
    msr     cnthctl_el2, x1
    msr     cntvoff_el2, xzr
    mov     x1, #0x0800                 // SCTLR_EL1: MMU and caches off (RES1 bits set)
    movk    x1, #0x30D0, lsl #16
    msr     sctlr_el1, x1
    mov     x1, #0x3C5                  // SPSR_EL2: return to EL1h, D/A/I/F masked
    msr     spsr_el2, x1
    adr     x1, el1_main
    msr     elr_el2, x1
    eret

el1_main:
    mov     x1, #0x0800                 // known state: MMU, caches, alignment checks off
    movk    x1, #0x30D0, lsl #16
    msr     sctlr_el1, x1

    adrp    x1, exception_vectors       // install the exception vector table
    add     x1, x1, :lo12:exception_vectors
    msr     vbar_el1, x1

    mov     x1, #0x300000               // CPACR_EL1.FPEN = 0b11: no trap on FP/SIMD
    msr     cpacr_el1, x1
    isb

    adrp    x1, __stack_top             // boot stack
    add     x1, x1, :lo12:__stack_top
    mov     sp, x1

    adrp    x1, __bss_start             // zero .bss
    add     x1, x1, :lo12:__bss_start
    adrp    x2, __bss_end
    add     x2, x2, :lo12:__bss_end
1:  cmp     x1, x2
    b.hs    2f
    str     xzr, [x1], #8
    b       1b

2:  mov     x0, x19                     // arg 0: device tree pointer
    bl      rust_entry                  // arch/aarch64/mod.rs, never returns

park:
    wfe
    b       park