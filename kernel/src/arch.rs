use core::arch::asm;

pub unsafe fn outb(port: u16, val: u8) {
    unsafe { asm!("out dx, al", in("dx") port, in("al") val, options(nomem, nostack)) };
}

pub unsafe fn inw(port: u16) -> u16 {
    let val: u16;
    unsafe { asm!("in ax, dx", in("dx") port, out("ax") val, options(nomem, nostack)) };
    val
}

pub unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    unsafe { asm!("in al, dx", in("dx") port, out("al") val, options(nomem, nostack)) };
    val
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

pub fn sleep_ms(ms: u32) {
    let mut remaining = ms;
    while remaining > 0 {
        let chunk = if remaining > 10 { 10 } else { remaining };
        pit_wait_ticks((PIT_HZ * chunk / 1000) as u16);
        remaining -= chunk;
    }
}

pub unsafe fn outw(port: u16, val: u16) {
    unsafe { asm!("out dx, ax", in("dx") port, in("ax") val, options(nomem, nostack)) };
}

pub fn reboot() -> ! {
    unsafe {
        // 1) Keyboard controller reset line (wait until its input buffer is empty)
        let mut tries = 0;
        while inb(0x64) & 0x02 != 0 && tries < 100_000 {
            tries += 1;
        }
        outb(0x64, 0xFE);

        // 2) Reset control register (works on most chipsets, QEMU included)
        outb(0xCF9, 0x06);

        // 3) Last resort: triple fault by loading an empty IDT and raising an exception
        let empty_idt = [0u8; 10];
        asm!("lidt [{}]", "int3", in(reg) &empty_idt, options(noreturn));
    }
}

pub fn shutdown() -> ! {
    unsafe {
        outw(0x604, 0x2000);  // QEMU (modern ACPI PM port)
        outw(0xB004, 0x2000); // QEMU (older) / Bochs
        outw(0x4004, 0x3400); // VirtualBox
        asm!("cli");
    }
    // Real hardware needs ACPI to power off, so halt forever instead
    loop {
        unsafe { asm!("hlt") };
    }
}

// pub fn reboot() -> ! {
//     unsafe { outb(0x64, 0xFE) }; // pulse the CPU reset line via the keyboard controller
//     loop {
//         unsafe { asm!("hlt") };
//     }
// }