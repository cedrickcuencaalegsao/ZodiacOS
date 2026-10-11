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