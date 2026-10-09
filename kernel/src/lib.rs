#![no_std]
#![no_main]

use core::arch::asm;
use core::panic::PanicInfo;

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        unsafe { asm!("hlt") };
    }
}

// ---- Port I/O ---------------------------------------------------------------
unsafe fn outb(port: u16, val: u8) {
    unsafe { asm!("out dx, al", in("dx") port, in("al") val, options(nomem, nostack)) };
}

unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    unsafe { asm!("in al, dx", in("dx") port, out("al") val, options(nomem, nostack)) };
    val
}

// ---- Timing (PIT channel 2, polled) -----------------------------------------
const PIT_HZ: u32 = 1_193_182;

/// Wait up to ~54 ms using one PIT channel 2 countdown.
fn pit_wait_ticks(ticks: u16) {
    unsafe {
        // Gate on (bit 0), speaker off (bit 1)
        let v = inb(0x61);
        outb(0x61, (v & 0xFC) | 0x01);

        // Channel 2, lobyte/hibyte, mode 0 (interrupt on terminal count), binary
        outb(0x43, 0xB0);
        outb(0x42, (ticks & 0xFF) as u8);
        outb(0x42, (ticks >> 8) as u8);

        // Bit 5 of port 0x61 goes high when the countdown hits zero
        while inb(0x61) & 0x20 == 0 {}
    }
}

fn sleep_ms(ms: u32) {
    // Split into 10 ms chunks (11,931 ticks) so the 16-bit counter never overflows
    let mut remaining = ms;
    while remaining > 0 {
        let chunk = if remaining > 10 { 10 } else { remaining };
        pit_wait_ticks((PIT_HZ * chunk / 1000) as u16);
        remaining -= chunk;
    }
}

// ---- VGA text mode ----------------------------------------------------------
const VGA_BUFFER: *mut u8 = 0xb8000 as *mut u8;
const VGA_WIDTH: usize = 80;
const VGA_HEIGHT: usize = 25;

fn put_char(row: usize, col: usize, ch: u8, attr: u8) {
    let offset = (row * VGA_WIDTH + col) * 2;
    unsafe {
        VGA_BUFFER.add(offset).write_volatile(ch);
        VGA_BUFFER.add(offset + 1).write_volatile(attr);
    }
}

fn print_at(row: usize, col: usize, message: &[u8], attr: u8) {
    for (i, &byte) in message.iter().enumerate() {
        put_char(row, col + i, byte, attr);
    }
}

fn clear_screen() {
    for row in 0..VGA_HEIGHT {
        for col in 0..VGA_WIDTH {
            put_char(row, col, b' ', 0x0F);
        }
    }
}

fn disable_cursor() {
    unsafe {
        outb(0x3D4, 0x0A); // select "cursor start" register
        outb(0x3D5, 0x20); // bit 5 = cursor disable
    }
}

#[allow(dead_code)]
fn enable_cursor() {
    unsafe {
        outb(0x3D4, 0x0A);
        outb(0x3D5, 0x0E); // start scanline 14
        outb(0x3D4, 0x0B);
        outb(0x3D5, 0x0F); // end scanline 15
    }
}

// ---- Loading animation ------------------------------------------------------
fn loading_animation(total_ms: u32) {
    const BAR_LEN: usize = 20;
    const SPINNER: [u8; 4] = [b'|', b'/', b'-', b'\\'];
    const ROW: usize = 12;
    const COL: usize = 30;

    print_at(ROW, COL, b"Loading ZodiacOS", 0x0F);
    put_char(ROW + 1, COL, b'[', 0x07);
    put_char(ROW + 1, COL + BAR_LEN + 1, b']', 0x07);

    // 3 sub-steps per bar segment, each with its own spinner frame
    let step_ms = total_ms / (BAR_LEN as u32 * 3);
    let mut frame = 0;

    for i in 0..BAR_LEN {
        for _ in 0..3 {
            put_char(ROW, COL + 17, SPINNER[frame % 4], 0x0E); // yellow spinner
            frame += 1;
            sleep_ms(step_ms);
        }
        put_char(ROW + 1, COL + 1 + i, 0xDB, 0x0A); // full block, light green.
    }
}

// ---- Entry point ------------------------------------------------------------
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    disable_cursor();
    clear_screen();
    loading_animation(3000);

    clear_screen();
    print_at(0, 0, b"Hello World from ZodiacOS!", 0x0F);

    loop {
        unsafe { asm!("hlt") };
    }
}