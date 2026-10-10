use crate::{arch, keyboard, vga};
use crate::{kprintln, slogln};
use core::arch::asm;
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, Ordering::SeqCst};

static PANICKING: AtomicBool = AtomicBool::new(false);

const PANIC_ATTR: u8 = 0x4F; // white on red
const TITLE_ATTR: u8 = 0x4E; // yellow on red

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    unsafe { asm!("cli") };

    // A panic while already panicking: skip the screen so we can't loop forever
    if PANICKING.swap(true, SeqCst) {
        slogln!("[PANIC] double panic, halting");
        halt();
    }

    slogln!("[PANIC] {}", info);

    begin_screen("KERNEL PANIC");
    if let Some(loc) = info.location() {
        kprintln!("at {}:{}:{}", loc.file(), loc.line(), loc.column());
    }
    kprintln!();
    kprintln!("{}", info.message());
    finish()
}

/// Clears the screen to red and prints the title. Shared with the exception handlers.
pub fn begin_screen(title: &str) {
    vga::disable_cursor();
    vga::set_color(PANIC_ATTR);
    vga::clear();
    vga::set_color(TITLE_ATTR);
    kprintln!("*** {} ***", title);
    kprintln!();
    vga::set_color(PANIC_ATTR);
}

/// Prints the footer, then waits for R (polled, interrupts are off) to restart.
pub fn finish() -> ! {
    kprintln!();
    kprintln!("System halted. Press R to restart.");
    loop {
        if let Some(c) = keyboard::read_char() {
            if c == b'r' || c == b'R' {
                arch::reboot();
            }
        }
        core::hint::spin_loop();
    }
}

fn halt() -> ! {
    loop {
        unsafe { asm!("hlt") };
    }
}