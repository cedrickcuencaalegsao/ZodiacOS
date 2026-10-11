use crate::arch::{inb, outb};
use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering::Relaxed};

const COM1: u16 = 0x3F8;

static READY: AtomicBool = AtomicBool::new(false);

pub fn init() {
    unsafe {
        outb(COM1 + 1, 0x00); // disable UART interrupts
        outb(COM1 + 3, 0x80); // DLAB on: set the baud divisor
        outb(COM1, 0x01);     // divisor 1 = 115200 baud (low byte)
        outb(COM1 + 1, 0x00); //                          (high byte)
        outb(COM1 + 3, 0x03); // DLAB off, 8 data bits, no parity, 1 stop bit
        outb(COM1 + 2, 0xC7); // enable and clear the FIFO
        outb(COM1 + 4, 0x1E); // loopback mode for a self-test
        outb(COM1, 0xAE);
        if inb(COM1) != 0xAE {
            return; // no working UART, logging stays disabled
        }
        outb(COM1 + 4, 0x0F); // normal operation
    }
    READY.store(true, Relaxed);
}

pub fn write_byte(b: u8) {
    if !READY.load(Relaxed) {
        return;
    }
    unsafe {
        // Wait (bounded) for the transmit buffer to empty
        for _ in 0..100_000 {
            if inb(COM1 + 5) & 0x20 != 0 {
                break;
            }
        }
        outb(COM1, b);
    }
}

pub struct Writer;

impl fmt::Write for Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            if b == b'\n' {
                write_byte(b'\r'); // terminals expect CR+LF
            }
            write_byte(b);
        }
        Ok(())
    }
}

#[macro_export]
macro_rules! slog {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let _ = write!($crate::serial::Writer, $($arg)*);
    }};
}

#[macro_export]
macro_rules! slogln {
    () => ($crate::slog!("\n"));
    ($($arg:tt)*) => ($crate::slog!("{}\n", format_args!($($arg)*)));
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => ($crate::slogln!("[INFO ] {}", format_args!($($arg)*)));
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => ($crate::slogln!("[WARN ] {}", format_args!($($arg)*)));
}

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => ($crate::slogln!("[ERROR] {}", format_args!($($arg)*)));
}