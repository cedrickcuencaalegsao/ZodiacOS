//! arch/aarch64/cpu.rs  -- the arch:: functions the scheduler, locks and shell expect.
//! Re-export these from arch/mod.rs for aarch64 (see the notes in the chat).

use core::arch::asm;

/// The boot thread's kernel stack top is only needed on x86 (it feeds the TSS).
pub const BOOT_KSTACK_TOP: u64 = 0;

pub fn interrupts_enabled() -> bool {
    let daif: u64;
    unsafe { asm!("mrs {}, daif", out(reg) daif, options(nomem, nostack)) };
    daif & (1 << 7) == 0 // I bit clear = IRQs enabled
}

/// Masks IRQs and returns whether they were enabled before. These asm blocks
/// deliberately do not say `nomem`, so the compiler won't move memory accesses
/// out of the protected region.
pub fn save_and_disable_interrupts() -> bool {
    let was = interrupts_enabled();
    unsafe { asm!("msr daifset, #2", options(nostack)) };
    was
}

pub fn restore_interrupts(was_enabled: bool) {
    if was_enabled {
        unsafe { asm!("msr daifclr, #2", options(nostack)) };
    }
}

/// x86 points the TSS at the new task's kernel stack here. On aarch64 nothing is needed:
/// SP_EL1 is a real register that keeps its value while a task runs at EL0, and each
/// task returns to EL0 from the same stack position it entered from, so the next
/// exception from user mode lands on that task's own kernel stack.
#[inline]
pub fn set_kernel_stack(_top: u64) {}

/// Stops the CPU for good: all interrupts masked, wait for events forever.
pub fn halt() -> ! {
    unsafe { asm!("msr daifset, #0xf", options(nomem, nostack)) };
    loop {
        unsafe { asm!("wfe", options(nomem, nostack)) };
    }
}

/// Current exception level: 1 for the kernel, 2 if you booted at EL2.
pub fn current_el() -> u64 {
    let v: u64;
    unsafe { asm!("mrs {}, CurrentEL", out(reg) v, options(nomem, nostack)) };
    (v >> 2) & 3
}

/// Generic timer frequency in Hz (QEMU `virt` reports 62_500_000).
pub fn timer_frequency() -> u64 {
    let v: u64;
    unsafe { asm!("mrs {}, cntfrq_el0", out(reg) v, options(nomem, nostack)) };
    v
}

fn counter() -> u64 {
    let v: u64;
    // isb first so the read isn't speculated ahead of earlier instructions
    unsafe { asm!("isb", "mrs {}, cntpct_el0", out(reg) v, options(nomem, nostack)) };
    v
}

/// Busy-waits on the free-running counter. Works before interrupts, the GIC or the
/// scheduler are set up. Use `interrupts::sleep_ms` (idle between ticks) once they are.
pub fn sleep_ms(ms: u64) {
    let wait = timer_frequency() * ms / 1000;
    let start = counter();
    while counter().wrapping_sub(start) < wait {
        unsafe { asm!("yield", options(nomem, nostack)) };
    }
}

// ---- power control through PSCI (QEMU `virt` and most boards) ----------------------
const PSCI_SYSTEM_OFF: u64 = 0x8400_0008;
const PSCI_SYSTEM_RESET: u64 = 0x8400_0009;

fn psci(function: u64) -> ! {
    // QEMU `virt` uses the HVC conduit unless it runs with EL3 firmware, in which case use `smc #0`.
    unsafe { asm!("hvc #0", in("x0") function, options(nostack)) };
    loop {
        unsafe { asm!("wfi", options(nomem, nostack)) };
    }
}

pub fn reboot() -> ! {
    psci(PSCI_SYSTEM_RESET)
}

pub fn shutdown() -> ! {
    psci(PSCI_SYSTEM_OFF)
}