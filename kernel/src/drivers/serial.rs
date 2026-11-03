use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering::Relaxed};

static READY: AtomicBool = AtomicBool::new(false);

// ===========================================================================
// x86_64: 16550 UART on COM1 (port I/O)
// ===========================================================================
#[cfg(target_arch = "x86_64")]
mod backend {
    use super::READY;
    use crate::arch::{inb, outb};
    use core::sync::atomic::Ordering::Relaxed;

    const COM1: u16 = 0x3F8;

    pub fn init() {
        outb(COM1 + 1, 0x00); // disable UART interrupts
        outb(COM1 + 3, 0x80); // DLAB on: set the baud divisor
        outb(COM1, 0x01); // divisor 1 = 115200 baud (low byte)
        outb(COM1 + 1, 0x00); //                          (high byte)
        outb(COM1 + 3, 0x03); // DLAB off, 8 data bits, no parity, 1 stop bit
        outb(COM1 + 2, 0xC7); // enable and clear the FIFO
        outb(COM1 + 4, 0x1E); // loopback mode for a self-test
        outb(COM1, 0xAE);
        if inb(COM1) != 0xAE {
            return; // no working UART, logging stays disabled
        }
        outb(COM1 + 4, 0x0F); // normal operation

        READY.store(true, Relaxed);
    }

    pub fn write_byte(b: u8) {
        // Wait (bounded) for the transmit buffer to empty
        for _ in 0..100_000 {
            if inb(COM1 + 5) & 0x20 != 0 {
                break;
            }
        }
        outb(COM1, b);
    }

    pub fn read_byte() -> Option<u8> {
        if inb(COM1 + 5) & 1 != 0 {
            Some(inb(COM1))
        } else {
            None
        }
    }
}

// ===========================================================================
// aarch64: ARM PL011 UART (memory-mapped). QEMU `virt` puts it at 0x0900_0000.
// ===========================================================================
#[cfg(target_arch = "aarch64")]
mod backend {
    use super::READY;
    use core::ptr::{read_volatile, write_volatile};
    use core::sync::atomic::{AtomicUsize, Ordering::Relaxed};

    /// Where the UART registers are reachable from the CPU right now. Starts as the
    /// physical address (fine while the MMU is off or identity-mapped); once the
    /// direct map is up, call `remap` with `PHYS_OFFSET + 0x0900_0000`.
    static BASE: AtomicUsize = AtomicUsize::new(0x0900_0000);

    const DR: usize = 0x00; // data
    const FR: usize = 0x18; // flags
    const IBRD: usize = 0x24; // integer baud divisor
    const FBRD: usize = 0x28; // fractional baud divisor
    const LCR_H: usize = 0x2C; // line control
    const CR: usize = 0x30; // control
    const IMSC: usize = 0x38; // interrupt mask
    const ICR: usize = 0x44; // interrupt clear

    const FR_TXFF: u32 = 1 << 5; // transmit FIFO full
    const FR_RXFE: u32 = 1 << 4; // receive FIFO empty

    pub fn remap(virt_base: usize) {
        BASE.store(virt_base, Relaxed);
    }

    fn rd(off: usize) -> u32 {
        unsafe { read_volatile((BASE.load(Relaxed) + off) as *const u32) }
    }

    fn wr(off: usize, v: u32) {
        unsafe { write_volatile((BASE.load(Relaxed) + off) as *mut u32, v) }
    }

    pub fn init() {
        wr(CR, 0); // disable while configuring
        wr(ICR, 0x7FF); // clear pending interrupts
        wr(IMSC, 0); // no UART interrupts (polled)
        // 115200 baud from a 24 MHz UART clock (QEMU ignores this; real boards use it):
        // 24_000_000 / (16 * 115200) = 13.02 -> IBRD 13, FBRD 1
        wr(IBRD, 13);
        wr(FBRD, 1);
        wr(LCR_H, 0x70); // 8 data bits, no parity, 1 stop bit, FIFOs on
        wr(CR, 0x301); // UARTEN | TXE | RXE
        READY.store(true, Relaxed);
    }

    pub fn write_byte(b: u8) {
        // Wait (bounded) for room in the transmit FIFO
        for _ in 0..100_000 {
            if rd(FR) & FR_TXFF == 0 {
                break;
            }
        }
        wr(DR, b as u32);
    }

    pub fn read_byte() -> Option<u8> {
        if rd(FR) & FR_RXFE == 0 {
            Some((rd(DR) & 0xFF) as u8)
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Shared API
// ---------------------------------------------------------------------------

pub fn init() {
    backend::init();
}

/// aarch64 only: switch the UART to a new virtual address after the MMU/direct map is set up.
#[cfg(target_arch = "aarch64")]
pub fn remap(virt_base: usize) {
    backend::remap(virt_base);
}

pub fn write_byte(b: u8) {
    if !READY.load(Relaxed) {
        return;
    }
    backend::write_byte(b);
}

/// Non-blocking read from the serial port.
pub fn read_byte() -> Option<u8> {
    if !READY.load(Relaxed) {
        return None;
    }
    backend::read_byte()
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
        let _ = write!($crate::drivers::serial::Writer, $($arg)*);
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