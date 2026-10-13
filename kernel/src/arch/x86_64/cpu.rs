use super::port_io::{inb, outb, outw};
use core::arch::asm;

pub fn halt() -> ! {
    loop {
        unsafe { asm!("hlt") };
    }
}

const PIT_HZ: u32 = 1_193_182;

fn pit_wait_ticks(ticks: u16) {
    unsafe {
        let v = inb(0x61);
        outb(0x61, (v & 0xFC) | 0x01);
        outb(0x43, 0xB0);
        outb(0x42, (ticks & 0xFF) as u8);
        outb(0x42, (ticks >> 8) as u8);
        while inb(0x61) & 0x20 == 0 {}
    }
}

/// Busy-wait delay (PIT channel 2). For early boot only: it needs no interrupts,
/// but it burns the CPU. Use `interrupts::sleep_ms` once interrupts are on.
pub fn sleep_ms(ms: u32) {
    let mut remaining = ms;
    while remaining > 0 {
        let chunk = if remaining > 10 { 10 } else { remaining };
        pit_wait_ticks((PIT_HZ * chunk / 1000) as u16);
        remaining -= chunk;
    }
}

pub fn reboot() -> ! {
    unsafe {
        let mut tries = 0;
        while inb(0x64) & 0x02 != 0 && tries < 100_000 {
            tries += 1;
        }
        outb(0x64, 0xFE); // keyboard controller reset line
        outb(0xCF9, 0x06); // reset control register
        let empty_idt = [0u8; 10];
        asm!("lidt [{}]", "int3", in(reg) &empty_idt, options(noreturn)); // triple fault
    }
}

pub fn shutdown() -> ! {
    unsafe {
        outw(0x604, 0x2000); // QEMU
        outw(0xB004, 0x2000); // older QEMU / Bochs
        outw(0x4004, 0x3400); // VirtualBox
        asm!("cli");
    }
    halt()
}

pub fn interrupts_enabled() -> bool{
    let flags: u64;
    unsafe { asm!("pushfq", "pop {}", out(reg) flags)};
    flags & (1 << 9) != 0
}

pub fn disable_interrupts() {
    unsafe { asm!("cli", options(nostack)) };
} 

pub fn enable_interrupts(){
    unsafe { asm!("sti", options(nostack)) };
}

/// Disable interrupts and reports whether they were on before.
pub fn save_and_disable_interrupts() -> bool {
    let was_enabled = interrupts_enabled();
    disable_interrupts();
    was_enabled
}

pub fn restore_interrupts(was_enabled: bool) {
    if was_enabled {
	enable_interrupts();
    }
}

