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

/// Integer square root (floor), no floats needed.
fn isqrt(n: i32) -> i32 {
    if n <= 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
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

    // ---- Graphics primitives (splash / HD UI) ------------------------------------

    /// Blend `src` over `dst` with alpha 0..=255 (colors are 0xRRGGBB).
    #[inline]
    fn blend(dst: u32, src: u32, a: u32) -> u32 {
        let ia = 255 - a;
        let ch = |shift: u32| {
            let s = (src >> shift) & 0xFF;
            let d = (dst >> shift) & 0xFF;
            ((s * a + d * ia) / 255) << shift
        };
        ch(16) | ch(8) | ch(0)
    }

    /// Plot one pixel with optional transparency, clipped to the screen.
    #[inline]
    fn put_px(&self, x: usize, y: usize, color: u32, alpha: u32) {
        if x >= self.width || y >= self.height {
            return;
        }
        unsafe {
            let p = self.row_ptr(y).add(x);
            let out = if alpha >= 255 {
                color
            } else {
                Self::blend(p.read_volatile(), color, alpha)
            };
            p.write_volatile(out);
        }
    }

    /// Fill a horizontal span with optional transparency.
    fn span(&self, x: usize, y: usize, w: usize, color: u32, alpha: u32) {
        if y >= self.height || x >= self.width {
            return;
        }
        let w = w.min(self.width - x);
        if alpha >= 255 {
            unsafe { core::slice::from_raw_parts_mut(self.row_ptr(y).add(x), w).fill(color) };
        } else {
            for i in 0..w {
                self.put_px(x + i, y, color, alpha);
            }
        }
    }

    /// Vertical gradient, top -> bottom (0xRRGGBB).
    pub fn fill_gradient(&self, x: usize, y: usize, w: usize, h: usize, top: u32, bottom: u32) {
        if h == 0 {
            return;
        }
        let denom = (h - 1).max(1);
        for i in 0..h {
            let t = (i * 255 / denom) as u32;
            self.span(x, y + i, w, Self::blend(top, bottom, t), 255);
        }
    }

    /// Anti-aliased rounded rectangle (4x4 supersampled corners).
    pub fn fill_round_rect(
        &self,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
        r: usize,
        color: u32,
        alpha: u8,
    ) {
        let r = r.min(w / 2).min(h / 2);
        let a = alpha as u32;
        for row in 0..h {
            let py = y + row;
            let in_corner_rows = row < r || row >= h - r;
            if !in_corner_rows {
                self.span(x, py, w, color, a);
                continue;
            }
            self.span(x + r, py, w - 2 * r, color, a);

            // circle centre (pixel coords) for this row's corner
            let cy = if row < r {
                (y + r) as i32
            } else {
                (y + h - r) as i32
            };
            let rad = r as i32 * 8;
            for i in 0..r {
                for &(px, cx) in &[(x + i, (x + r) as i32), (x + w - 1 - i, (x + w - r) as i32)] {
                    let mut hits = 0u32;
                    for sy in 0..4i32 {
                        for sx in 0..4i32 {
                            let ux = px as i32 * 8 + 2 * sx + 1 - cx * 8;
                            let uy = py as i32 * 8 + 2 * sy + 1 - cy * 8;
                            if ux * ux + uy * uy <= rad * rad {
                                hits += 1;
                            }
                        }
                    }
                    if hits > 0 {
                        self.put_px(px, py, color, a * hits / 16);
                    }
                }
            }
        }
    }

    /// Anti-aliased filled circle with alpha (edge pixels use 4x4 supersampling).
    pub fn fill_circle(&self, cx: usize, cy: usize, r: usize, color: u32, alpha: u8) {
        let (cx, cy, r) = (cx as i32, cy as i32, r as i32);
        let a = alpha as u32;
        for y in (cy - r - 1)..=(cy + r + 1) {
            if y < 0 || y as usize >= self.height {
                continue;
            }
            let dy = y - cy;
            let outer_sq = (r + 1) * (r + 1) - dy * dy;
            if outer_sq < 0 {
                continue;
            }
            let outer = isqrt(outer_sq);
            let inner_sq = (r - 1) * (r - 1) - dy * dy;
            let inner = if inner_sq >= 0 { isqrt(inner_sq) } else { -1 };

            // fully covered middle part: one fast span
            let (left_end, right_start) = if inner >= 0 {
                let x0 = (cx - inner).max(0);
                let x1 = cx + inner;
                if x1 >= 0 {
                    self.span(x0 as usize, y as usize, (x1 - x0 + 1) as usize, color, a);
                }
                (cx - inner - 1, cx + inner + 1)
            } else {
                (cx + outer, cx + outer + 1) // whole row is edge
            };

            let edge = |x: i32| {
                if x < 0 {
                    return;
                }
                let mut hits = 0u32;
                for sy in 0..4i32 {
                    for sx in 0..4i32 {
                        let ux = x * 8 + 2 * sx + 1 - (cx * 8 + 4);
                        let uy = y * 8 + 2 * sy + 1 - (cy * 8 + 4);
                        if ux * ux + uy * uy <= (r * 8) * (r * 8) {
                            hits += 1;
                        }
                    }
                }
                if hits > 0 {
                    self.put_px(x as usize, y as usize, color, a * hits / 16);
                }
            };
            for x in (cx - outer)..=left_end {
                edge(x);
            }
            for x in right_start..=(cx + outer) {
                edge(x);
            }
        }
    }

    /// Alpha-blends an 8-bit coverage mask (anti-aliased glyphs) tinted with `color`.
    pub fn blit_mask(&self, x: usize, y: usize, w: usize, h: usize, mask: &[u8], color: u32) {
        if mask.len() < w * h {
            return;
        }
        for row in 0..h {
            for col in 0..w {
                let a = mask[row * w + col] as u32;
                if a != 0 {
                    self.put_px(x + col, y + row, color, a);
                }
            }
        }
    }

    /// 8x16 glyph scaled by an integer factor, transparent background.
    pub fn draw_glyph_scaled(
        &self,
        x: usize,
        y: usize,
        glyph: &[u8; 16],
        scale: usize,
        color: u32,
    ) {
        for (row, bits) in glyph.iter().enumerate() {
            for col in 0..8 {
                if bits & (0x80 >> col) != 0 {
                    self.fill_rect(x + col * scale, y + row * scale, scale, scale, color);
                }
            }
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

/// The captured 8x16 console font (256 glyphs), used by the desktop for its text.
pub fn font_table() -> &'static [[u8; 16]; 256] {
    unsafe { &*FONT.0.get() }
}

// ---- The active framebuffer, so the desktop can find it later ----------------
struct FbSlot(UnsafeCell<Option<Framebuffer>>);
unsafe impl Sync for FbSlot {}

static ACTIVE_FB: FbSlot = FbSlot(UnsafeCell::new(None));

/// The framebuffer, if a graphics mode was set up successfully.
pub fn get() -> Option<Framebuffer> {
    unsafe { *ACTIVE_FB.0.get() }
}

/// How many full screens the video memory can flip between (2 = double buffering works).
/// Page 1 starts `height * pitch` bytes after `Framebuffer::base`.
pub fn pages() -> usize {
    super::bochs_vga::pages()
}

/// Makes screen `page` the one the monitor shows; drawing on the other page is invisible.
pub fn show_page(page: usize) {
    super::bochs_vga::show_page(page)
}

/// The BIOS keeps its text font in VGA plane 2, 32 bytes per character.
fn capture_vga_font() -> Result<(), &'static str> {
    unsafe {
        // save the registers we change
        outb(0x3C4, 0x02);
        let seq2 = inb(0x3C5);
        outb(0x3C4, 0x04);
        let seq4 = inb(0x3C5);
        outb(0x3CE, 0x04);
        let gc4 = inb(0x3CF);
        outb(0x3CE, 0x05);
        let gc5 = inb(0x3CF);
        outb(0x3CE, 0x06);
        let gc6 = inb(0x3CF);

        // expose plane 2 at 0xA0000 for reading
        outb(0x3C4, 0x02);
        outb(0x3C5, 0x04);
        outb(0x3C4, 0x04);
        outb(0x3C5, 0x07);
        outb(0x3CE, 0x04);
        outb(0x3CF, 0x02);
        outb(0x3CE, 0x05);
        outb(0x3CF, 0x00);
        outb(0x3CE, 0x06);
        outb(0x3CF, 0x00);

        let src = 0xA0000 as *const u8;
        let table = &mut *FONT.0.get();
        for ch in 0..256usize {
            for row in 0..16usize {
                table[ch][row] = src.add(ch * 32 + row).read_volatile();
            }
        }

        // put text mode back exactly as it was
        outb(0x3C4, 0x02);
        outb(0x3C5, seq2);
        outb(0x3C4, 0x04);
        outb(0x3C5, seq4);
        outb(0x3CE, 0x04);
        outb(0x3CF, gc4);
        outb(0x3CE, 0x05);
        outb(0x3CF, gc5);
        outb(0x3CE, 0x06);
        outb(0x3CF, gc6);

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
    unsafe { *ACTIVE_FB.0.get() = Some(fb) }; // remember it for the desktop
    crate::console::fbcon::activate(fb, crate::vga::attr());
    Ok(())
}
