use core::arch::asm;

pub fn halt() -> ! {
    loop {
        unsafe { asm!("wfe", options(nomem, nostack)) };
    }
}

pub fn current_el() -> u64 {
    let v: u64;
    unsafe { asm!("mrs {}, CurrentEL", out(reg) v, options(nomem, nostack)) };
    v >> 2
}

pub fn timer_frequency() -> u64 {
    let f: u64;
    unsafe { asm!("mrs {}, cntfrq_el0", out(reg) f, options(nomem, nostack)) };
    f
}

fn counter() -> u64 {
    let c: u64;
    unsafe { asm!("isb", "mrs {}, cntvct_el0", out(reg) c, options(nomem, nostack)) };
    c
}

/// Busy-wait delay using the architectural generic timer (works on any ARMv8 system).
pub fn sleep_ms(ms: u32) {
    let ticks = timer_frequency() * ms as u64 / 1000;
    let start = counter();
    while counter().wrapping_sub(start) < ticks {
        core::hint::spin_loop();
    }
}

/// PSCI power control. QEMU's "virt" machine answers on the HVC conduit.
fn psci(function: u64) -> ! {
    unsafe {
        asm!(
            "hvc #0",
            inout("x0") function => _,
            out("x1") _,
            out("x2") _,
            out("x3") _,
            options(nomem, nostack)
        )
    };
    halt()
}

pub fn shutdown() -> ! {
    psci(0x8400_0008) // PSCI SYSTEM_OFF
}

pub fn reboot() -> ! {
    psci(0x8400_0009) // PSCI SYSTEM_RESET
}

pub fn interrupts_enabled() -> bool {
    let daif: u64;
    unsafe { asm!("mrs {}, daif", out(reg), options(nomem, nostack)) };
    diaf & (1 << 7) == 0
}

pub fn disabled_interrupts() {
    unsafe { asm!("msr daifset, #2", options(nomem, nostack)) };
}

pub fn enable_interrupts() {
    unsafe { asm!("msr daifset, #2", options(nomem, nostack)) };
}

pub fn save_and_disable_interrupts() -> bool {
    let was_enabled = interrupts_enabled();
    disabled_interrupts();
    was_enabled
}

pub fn restore_interrupts(was_enabled: bool) {
    if was_enabled {
	enable_interrupts();
    }
} 
