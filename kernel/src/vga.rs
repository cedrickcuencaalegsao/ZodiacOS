use crate::arch::outb;
use crate::console::fbcon;
use core::fmt;
use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering::Relaxed};

// Text-mode geometry (only used until the framebuffer takes over)
pub const WIDTH: usize = 80;
pub const HEIGHT: usize = 25;
const BUFFER: *mut u8 = 0xb8000 as *mut u8;

static ROW: AtomicUsize = AtomicUsize::new(0);
static COL: AtomicUsize = AtomicUsize::new(0);
static ATTR: AtomicU8 = AtomicU8::new(0x0F);

/// Screen size in characters, whichever mode is active.
pub fn width() -> usize {
    if fbcon::active() { fbcon::cols() } else { WIDTH }
}

pub fn height() -> usize {
    if fbcon::active() { fbcon::rows() } else { HEIGHT }
}

pub fn attr() -> u8 {
    ATTR.load(Relaxed)
}

pub fn put_char(row: usize, col: usize, ch: u8, attr: u8) {
    if fbcon::active() {
        return fbcon::put_char(row, col, ch, attr);
    }
    if row >= HEIGHT || col >= WIDTH {
        return;
    }
    let offset = (row * WIDTH + col) * 2;
    unsafe {
        BUFFER.add(offset).write_volatile(ch);
        BUFFER.add(offset + 1).write_volatile(attr);
    }
}

pub fn print_at(row: usize, col: usize, msg: &[u8], attr: u8) {
    for (i, &b) in msg.iter().enumerate() {
        put_char(row, col + i, b, attr);
    }
}

pub fn set_color(attr: u8) {
    ATTR.store(attr, Relaxed);
    if fbcon::active() {
        fbcon::set_color(attr);
    }
}

pub fn clear() {
    if fbcon::active() {
        return fbcon::clear();
    }
    let attr = ATTR.load(Relaxed);
    for row in 0..HEIGHT {
        for col in 0..WIDTH {
            put_char(row, col, b' ', attr);
        }
    }
    ROW.store(0, Relaxed);
    COL.store(0, Relaxed);
    update_cursor();
}

fn scroll() {
    for row in 1..HEIGHT {
        for col in 0..WIDTH {
            let src = (row * WIDTH + col) * 2;
            let dst = ((row - 1) * WIDTH + col) * 2;
            unsafe {
                BUFFER.add(dst).write_volatile(BUFFER.add(src).read_volatile());
                BUFFER.add(dst + 1).write_volatile(BUFFER.add(src + 1).read_volatile());
            }
        }
    }
    let attr = ATTR.load(Relaxed);
    for col in 0..WIDTH {
        put_char(HEIGHT - 1, col, b' ', attr);
    }
}

pub fn put_byte(b: u8) {
    if fbcon::active() {
        return fbcon::put_byte(b);
    }
    let attr = ATTR.load(Relaxed);
    let mut row = ROW.load(Relaxed);
    let mut col = COL.load(Relaxed);

    match b {
        b'\n' => {
            col = 0;
            row += 1;
        }
        b'\r' => col = 0,
        8 => {
            if col > 0 {
                col -= 1;
            } else if row > 0 {
                row -= 1;
                col = WIDTH - 1;
            }
            put_char(row, col, b' ', attr);
        }
        _ => {
            put_char(row, col, b, attr);
            col += 1;
            if col >= WIDTH {
                col = 0;
                row += 1;
            }
        }
    }

    if row >= HEIGHT {
        scroll();
        row = HEIGHT - 1;
    }
    ROW.store(row, Relaxed);
    COL.store(col, Relaxed);
    update_cursor();
}

pub fn backspace() {
    put_byte(8);
}

// ---- Hardware cursor (text mode only; the framebuffer draws its own) ------------
pub fn update_cursor() {
    if fbcon::active() {
        return;
    }
    let pos = (ROW.load(Relaxed) * WIDTH + COL.load(Relaxed)) as u16;
    unsafe {
        outb(0x3D4, 0x0F);
        outb(0x3D5, (pos & 0xFF) as u8);
        outb(0x3D4, 0x0E);
        outb(0x3D5, (pos >> 8) as u8);
    }
}

pub fn disable_cursor() {
    if fbcon::active() {
        return fbcon::set_cursor(false);
    }
    unsafe {
        outb(0x3D4, 0x0A);
        outb(0x3D5, 0x20);
    }
}

pub fn enable_cursor() {
    if fbcon::active() {
        return fbcon::set_cursor(true);
    }
    unsafe {
        outb(0x3D4, 0x0A);
        outb(0x3D5, 0x0E);
        outb(0x3D4, 0x0B);
        outb(0x3D5, 0x0F);
    }
}

// ---- kprint! / kprintln! -----------------------------------------------------------
pub struct Writer;

impl fmt::Write for Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            put_byte(b);
        }
        Ok(())
    }
}

#[macro_export]
macro_rules! kprint {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let _ = write!($crate::vga::Writer, $($arg)*);
    }};
}

#[macro_export]
macro_rules! kprintln {
    () => ($crate::kprint!("\n"));
    ($($arg:tt)*) => ($crate::kprint!("{}\n", format_args!($($arg)*)));
}