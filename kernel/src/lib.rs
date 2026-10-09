#![no_std]
#![no_main]

use core::arch::asm;
use core::panic::PanicInfo;

mod arch;
mod ata;
mod base;
mod fs;
mod keyboard;
mod package;
mod time;
mod user;
mod vga;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    vga::set_color(0x4F); // white on red
    kprintln!("\nKERNEL PANIC: {}", info);
    loop {
        unsafe { asm!("hlt") };
    }
}

unsafe extern "C" {
    static __bss_start: u8;
    static __bss_end: u8;
}

/// objcopy -O binary doesn't include .bss, and the bootloader doesn't clear it.
unsafe fn zero_bss() {
    unsafe {
        let start = &raw const __bss_start as *mut u8;
        let end = &raw const __bss_end as *const u8;
        core::ptr::write_bytes(start, 0, end as usize - start as usize);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    unsafe { zero_bss() };

    vga::disable_cursor();
    vga::clear();
    base::splash::run(3000);

    vga::clear();
    vga::enable_cursor();
    kprintln!("ZodiacOS 0.1.0");
    kprintln!("Type 'help' for a list of commands.\n");

    base::shell::run()
}