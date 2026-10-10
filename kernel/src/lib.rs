#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

mod arch;
mod ata;
mod base;
mod fs;
mod idt;
mod keyboard;
mod package;
mod panic;
mod serial;
mod time;
mod user;
mod vga;

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
    serial::init();
    idt::init();
    log_info!("ZodiacOS kernel started");

    vga::disable_cursor();
    vga::clear();
    base::splash::run(3000);
    log_info!("splash finished");

    vga::clear();
    vga::enable_cursor();
    kprintln!("ZodiacOS 0.1.0");
    kprintln!("Type 'help' for a list of commands.\n");

    log_info!("starting shell");
    base::shell::run()
}