use crate::arch::{inb, outb};
use core::cell::UnsafeCell;

/// A linear 32-bit-per-pixel framebuffer (pixel = 0x00RRGGBB).
#[derive(Clone, Copy)]
pub struct Framebuffer {
    pub base: usize,
    pub width: usize,
    pub height: usize,
    pub pitch: usize, // bytes per row
}

impl Framebuffer {
    fn row_ptr(&self, y: usize) -> *mut u32 {
        (self.base + y * self.pitch) as *mut u32
    }

    pub fn fill_rect(&self, x: usize, y: usize, w: usize, h: usize, color: u32) {
        if x >= self.width || y >= self.height {
            return;
        }
        let w = w.min(self.width - x);
        let h = h.min(self.height - y);
        for row in y..y + h {
            let line = unsafe { core::slice::from_raw_parts_mut(self.row_ptr(row).add(x), w) };
            line.fill(color);
        }
    }

    /// Draws one 8x16 glyph with its top-left corner at pixel (x, y).
    pub fn draw_glyph(&self, x: usize, y: usize, glyph: &[u8; 16], fg: u32, bg: u32) {
        for (row, bits) in glyph.iter().enumerate() {
            let line = unsafe { self.row_ptr(y + row).add(x) };
            for col in 0..8 {
                let color = if bits & (0x80 >> col) != 0 { fg } else { bg };
                unsafe { line.add(col).write_volatile(color) };
            }
        }
    }

    /// Moves the whole picture up by `rows` pixel rows (the caller clears the bottom).
    pub fn scroll_up(&self, rows: usize) {
        unsafe {
            core::ptr::copy(
                self.row_ptr(rows) as *const u8,
                self.row_ptr(0) as *mut u8,
                (self.height - rows) * self.pitch,
            );
        }
    }
}

// ---- The 8x16 font, copied out of VGA memory before the mode switch ----------
struct FontTable(UnsafeCell<[[u8; 16]; 256]>);
unsafe impl Sync for FontTable {}

static FONT: FontTable = FontTable(UnsafeCell::new([[0; 16]; 256]));

pub fn glyph(ch: u8) -> &'static [u8; 16] {
    unsafe { &(*FONT.0.get())[ch as usize] }
}

/// The BIOS keeps its text font in VGA plane 2, 32 bytes per character.
fn capture_vga_font() -> Result<(), &'static str> {
    unsafe {
        // save the registers we change
        outb(0x3C4, 0x02); let seq2 = inb(0x3C5);
        outb(0x3C4, 0x04); let seq4 = inb(0x3C5);
        outb(0x3CE, 0x04); let gc4 = inb(0x3CF);
        outb(0x3CE, 0x05); let gc5 = inb(0x3CF);
        outb(0x3CE, 0x06); let gc6 = inb(0x3CF);

        // expose plane 2 at 0xA0000 for reading
        outb(0x3C4, 0x02); outb(0x3C5, 0x04);
        outb(0x3C4, 0x04); outb(0x3C5, 0x07);
        outb(0x3CE, 0x04); outb(0x3CF, 0x02);
        outb(0x3CE, 0x05); outb(0x3CF, 0x00);
        outb(0x3CE, 0x06); outb(0x3CF, 0x00);

        let src = 0xA0000 as *const u8;
        let table = &mut *FONT.0.get();
        for ch in 0..256usize {
            for row in 0..16usize {
                table[ch][row] = src.add(ch * 32 + row).read_volatile();
            }
        }

        // put text mode back exactly as it was
        outb(0x3C4, 0x02); outb(0x3C5, seq2);
        outb(0x3C4, 0x04); outb(0x3C5, seq4);
        outb(0x3CE, 0x04); outb(0x3CF, gc4);
        outb(0x3CE, 0x05); outb(0x3CF, gc5);
        outb(0x3CE, 0x06); outb(0x3CF, gc6);

        if table[b'A' as usize].iter().all(|&b| b == 0) {
            return Err("could not read the font from VGA memory");
        }
    }
    Ok(())
}

/// Switches to a graphics mode and moves the console onto it.
/// On any error the system simply stays in VGA text mode.
pub fn init(width: u16, height: u16) -> Result<(), &'static str> {
    capture_vga_font()?; // must happen before the mode switch
    let fb = super::bochs_vga::init(width, height)?;
    crate::console::fbcon::activate(fb, crate::vga::attr());
    Ok(())
}