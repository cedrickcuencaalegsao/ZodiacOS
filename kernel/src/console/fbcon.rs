#![allow(dead_code)]
use crate::drivers::framebuffer::{self, Framebuffer};
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering::Relaxed};

const MAX_COLS: usize = 240; // up to 1920 pixels wide
const MAX_ROWS: usize = 100; // up to 1600 pixels tall
const CELL_W: usize = 8;
const CELL_H: usize = 16;

/// The standard 16 VGA colors (attribute: low nibble = text, high nibble = background).
const PALETTE: [u32; 16] = [
    0x000000, 0x0000AA, 0x00AA00, 0x00AAAA, 0xAA0000, 0xAA00AA, 0xAA5500, 0xAAAAAA, 0x555555,
    0x5555FF, 0x55FF55, 0x55FFFF, 0xFF5555, 0xFF55FF, 0xFFFF55, 0xFFFFFF,
];

struct Shared<T>(UnsafeCell<T>);
unsafe impl<T> Sync for Shared<T> {}

// One entry per character cell: (attribute << 8) | character.
// The console has no lock on purpose: the panic screen must be able to print
// even if a panic interrupted the console itself. Single core, no IRQ printing.
static CELLS: Shared<[u16; MAX_COLS * MAX_ROWS]> =
    Shared(UnsafeCell::new([0; MAX_COLS * MAX_ROWS]));
static CONSOLE: Shared<Option<Console>> = Shared(UnsafeCell::new(None));
static ACTIVE: AtomicBool = AtomicBool::new(false);

fn cells() -> &'static mut [u16; MAX_COLS * MAX_ROWS] {
    unsafe { &mut *CELLS.0.get() }
}

struct Console {
    fb: Framebuffer,
    cols: usize,
    rows: usize,
    row: usize,
    col: usize,
    attr: u8,
    cursor_enabled: bool,
    cursor_drawn: bool,
}

impl Console {
    fn draw_cell(&self, r: usize, c: usize) {
        let v = cells()[r * MAX_COLS + c];
        let (ch, attr) = (v as u8, (v >> 8) as u8);
        let mut fg = PALETTE[(attr & 0x0F) as usize];
        let mut bg = PALETTE[(attr >> 4) as usize];
        if self.cursor_drawn && r == self.row && c == self.col {
            core::mem::swap(&mut fg, &mut bg); // block cursor = inverted cell
        }
        self.fb
            .draw_glyph(c * CELL_W, r * CELL_H, framebuffer::glyph(ch), fg, bg);
    }

    fn set_char(&mut self, r: usize, c: usize, ch: u8, attr: u8) {
        cells()[r * MAX_COLS + c] = ((attr as u16) << 8) | ch as u16;
        self.draw_cell(r, c);
    }

    fn hide_cursor(&mut self) {
        if self.cursor_drawn {
            self.cursor_drawn = false;
            self.draw_cell(self.row, self.col);
        }
    }

    fn show_cursor(&mut self) {
        if self.cursor_enabled && !self.cursor_drawn {
            self.cursor_drawn = true;
            self.draw_cell(self.row, self.col);
        }
    }

    fn background(&self) -> u32 {
        PALETTE[(self.attr >> 4) as usize]
    }

    fn clear(&mut self) {
        let blank = ((self.attr as u16) << 8) | b' ' as u16;
        let grid = cells();
        for r in 0..self.rows {
            for c in 0..self.cols {
                grid[r * MAX_COLS + c] = blank;
            }
        }
        self.fb
            .fill_rect(0, 0, self.fb.width, self.fb.height, self.background());
        self.row = 0;
        self.col = 0;
        self.cursor_drawn = false;
        self.show_cursor();
    }

    fn scroll(&mut self) {
        let blank = ((self.attr as u16) << 8) | b' ' as u16;
        let grid = cells();
        grid.copy_within(MAX_COLS..self.rows * MAX_COLS, 0);
        for c in 0..self.cols {
            grid[(self.rows - 1) * MAX_COLS + c] = blank;
        }
        self.fb.scroll_up(CELL_H);
        // clear the new last row (and any leftover pixels below the last full row)
        let top = (self.rows - 1) * CELL_H;
        self.fb.fill_rect(
            0,
            top,
            self.fb.width,
            self.fb.height - top,
            self.background(),
        );
    }

    fn put_byte(&mut self, b: u8) {
        self.hide_cursor();
        match b {
            b'\n' => {
                self.col = 0;
                self.row += 1;
            }
            b'\r' => self.col = 0,
            8 => {
                if self.col > 0 {
                    self.col -= 1;
                } else if self.row > 0 {
                    self.row -= 1;
                    self.col = self.cols - 1;
                }
                self.set_char(self.row, self.col, b' ', self.attr);
            }
            _ => {
                self.set_char(self.row, self.col, b, self.attr);
                self.col += 1;
                if self.col >= self.cols {
                    self.col = 0;
                    self.row += 1;
                }
            }
        }
        if self.row >= self.rows {
            self.scroll();
            self.row = self.rows - 1;
        }
        self.show_cursor();
    }
}

fn with<R>(f: impl FnOnce(&mut Console) -> R) -> Option<R> {
    unsafe { (*CONSOLE.0.get()).as_mut().map(f) }
}

// ---- Public API (called from vga.rs) -------------------------------------------

pub fn activate(fb: Framebuffer, attr: u8) {
    let console = Console {
        fb,
        cols: (fb.width / CELL_W).min(MAX_COLS),
        rows: (fb.height / CELL_H).min(MAX_ROWS),
        row: 0,
        col: 0,
        attr,
        cursor_enabled: true,
        cursor_drawn: false,
    };
    unsafe { *CONSOLE.0.get() = Some(console) };
    ACTIVE.store(true, Relaxed);
    with(|c| c.clear());
}

pub fn active() -> bool {
    ACTIVE.load(Relaxed)
}

pub fn cols() -> usize {
    with(|c| c.cols).unwrap_or(0)
}

pub fn rows() -> usize {
    with(|c| c.rows).unwrap_or(0)
}

/// (width px, height px, base address, columns, rows)
#[allow(dead_code)]
pub fn info() -> Option<(usize, usize, usize, usize, usize)> {
    with(|c| (c.fb.width, c.fb.height, c.fb.base, c.cols, c.rows))
}

/// Framebuffer size in pixels, or (0, 0) if the framebuffer console isn't active.
pub fn pixel_size() -> (usize, usize) {
    with(|c| (c.fb.width, c.fb.height)).unwrap_or((0, 0))
}

// ---- Pixel graphics (bypass the text grid; call vga::clear() afterwards) --------

/// Fill a pixel rectangle with 0xRRGGBB, clipped to the screen.
pub fn fill_px(x: usize, y: usize, w: usize, h: usize, rgb: u32) {
    with(|c| c.fb.fill_rect(x, y, w, h, rgb));
}

/// Same as `fill_px` with a gray level 0..=255.
pub fn fill_gray(x: usize, y: usize, w: usize, h: usize, v: u8) {
    fill_px(x, y, w, h, v as u32 * 0x01_01_01);
}

/// Vertical gradient from `top` to `bottom` (0xRRGGBB).
pub fn gradient(x: usize, y: usize, w: usize, h: usize, top: u32, bottom: u32) {
    with(|c| c.fb.fill_gradient(x, y, w, h, top, bottom));
}

/// Anti-aliased rounded rectangle with alpha 0..=255.
pub fn round_rect(x: usize, y: usize, w: usize, h: usize, r: usize, rgb: u32, alpha: u8) {
    with(|c| c.fb.fill_round_rect(x, y, w, h, r, rgb, alpha));
}

/// Anti-aliased filled circle with alpha 0..=255.
pub fn circle(cx: usize, cy: usize, r: usize, rgb: u32, alpha: u8) {
    with(|c| c.fb.fill_circle(cx, cy, r, rgb, alpha));
}

/// Width in pixels of `s` drawn with `text()` at the given integer scale.
pub fn text_width(s: &str, scale: usize) -> usize {
    s.len() * CELL_W * scale
}

/// Pixel-positioned text with a transparent background.
pub fn text(x: usize, y: usize, s: &str, scale: usize, rgb: u32) {
    with(|c| {
        for (i, b) in s.bytes().enumerate() {
            c.fb.draw_glyph_scaled(x + i * CELL_W * scale, y, framebuffer::glyph(b), scale, rgb);
        }
    });
}

/// Blend an 8-bit coverage mask (glyph) tinted with `rgb`.
pub fn blit_mask(x: usize, y: usize, w: usize, h: usize, mask: &[u8], rgb: u32) {
    with(|c| c.fb.blit_mask(x, y, w, h, mask, rgb));
}

// ---- Text grid -------------------------------------------------------------------

pub fn put_char(row: usize, col: usize, ch: u8, attr: u8) {
    with(|c| {
        if row < c.rows && col < c.cols {
            c.hide_cursor();
            c.set_char(row, col, ch, attr);
            c.show_cursor();
        }
    });
}

pub fn put_byte(b: u8) {
    with(|c| c.put_byte(b));
}

pub fn clear() {
    with(|c| c.clear());
}

pub fn set_color(attr: u8) {
    with(|c| c.attr = attr);
}

pub fn set_cursor(enabled: bool) {
    with(|c| {
        c.hide_cursor();
        c.cursor_enabled = enabled;
        c.show_cursor();
    });
}