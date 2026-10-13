//! Software renderer drawing into a pixel slice owned by the caller.
//! Colors are 0x00RRGGBB; `alpha` is 0..=255.
use crate::Font;

// Which corners of a rounded rectangle are rounded.
pub const TL: u8 = 1;
pub const TR: u8 = 2;
pub const BL: u8 = 4;
pub const BR: u8 = 8;
pub const ALL: u8 = 15;

pub struct Gfx<'a> {
    buf: &'a mut [u32],
    pub w: i32,
    pub h: i32,
    font: &'static Font,
}

pub fn text_width(s: &str, scale: i32) -> i32 {
    s.len() as i32 * 8 * scale
}

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

#[inline]
fn div255(v: u32) -> u32 {
    (v + 1 + (v >> 8)) >> 8
}

/// `src` over `dst` with coverage `a`.
#[inline]
pub fn blend(dst: u32, src: u32, a: u32) -> u32 {
    let ia = 255 - a;
    let r = div255(((src >> 16) & 0xFF) * a + ((dst >> 16) & 0xFF) * ia);
    let g = div255(((src >> 8) & 0xFF) * a + ((dst >> 8) & 0xFF) * ia);
    let b = div255((src & 0xFF) * a + (dst & 0xFF) * ia);
    (r << 16) | (g << 8) | b
}

/// For a rounded corner of radius `r`: the pixel row `d` rows from the edge starts
/// `inset` pixels in, and that first pixel is covered `alpha`/255.
fn corner(r: i32, d: i32) -> (i32, u32) {
    let r16 = r * 16;
    let dy16 = (r - d) * 16 - 8;
    let hw = isqrt((r16 * r16 - dy16 * dy16).max(0));
    let left16 = r16 - hw;
    (left16 / 16, ((16 - left16 % 16) as u32 * 255) / 16)
}

impl<'a> Gfx<'a> {
    pub fn new(buf: &'a mut [u32], w: usize, h: usize, font: &'static Font) -> Self {
        Gfx { buf, w: w as i32, h: h as i32, font }
    }

    pub fn px(&mut self, x: i32, y: i32, c: u32, a: u32) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h || a == 0 {
            return;
        }
        let i = (y * self.w + x) as usize;
        self.buf[i] = if a >= 255 { c } else { blend(self.buf[i], c, a) };
    }

    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: u32, a: u32) {
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + w).min(self.w);
        let y1 = (y + h).min(self.h);
        if x0 >= x1 || y0 >= y1 || a == 0 {
            return;
        }
        for yy in y0..y1 {
            let s = (yy * self.w + x0) as usize;
            let e = (yy * self.w + x1) as usize;
            let row = &mut self.buf[s..e];
            if a >= 255 {
                row.fill(c);
            } else {
                for p in row.iter_mut() {
                    *p = blend(*p, c, a);
                }
            }
        }
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: u32) {
        self.rect(x, y, w, h, c, 255);
    }

    /// Anti-aliased rounded rectangle with a vertical gradient (`top` -> `bot`).
    pub fn round_rect(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32, top: u32, bot: u32, alpha: u32) {
        self.round_rect_c(x, y, w, h, r, ALL, top, bot, alpha);
    }

    /// Same, but only the corners named in `flags` (TL | TR | BL | BR) are rounded.
    pub fn round_rect_c(
        &mut self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        r: i32,
        flags: u8,
        top: u32,
        bot: u32,
        alpha: u32,
    ) {
        if w <= 0 || h <= 0 {
            return;
        }
        let r = r.min(w / 2).min(h / 2).max(0);
        for row in 0..h {
            let col = if top == bot {
                top
            } else {
                blend(top, bot, (row * 255 / (h - 1).max(1)) as u32)
            };
            let from_top = row;
            let from_bot = h - 1 - row;
            let (il, al, ir, ar) = if r > 0 && from_top < r {
                let (i, a) = corner(r, from_top);
                (
                    if flags & TL != 0 { i } else { 0 },
                    if flags & TL != 0 { a } else { 255 },
                    if flags & TR != 0 { i } else { 0 },
                    if flags & TR != 0 { a } else { 255 },
                )
            } else if r > 0 && from_bot < r {
                let (i, a) = corner(r, from_bot);
                (
                    if flags & BL != 0 { i } else { 0 },
                    if flags & BL != 0 { a } else { 255 },
                    if flags & BR != 0 { i } else { 0 },
                    if flags & BR != 0 { a } else { 255 },
                )
            } else {
                (0, 255, 0, 255)
            };
            let mut sx = x + il;
            let mut ex = x + w - 1 - ir;
            if sx > ex {
                continue;
            }
            if al < 255 {
                self.px(sx, y + row, col, al * alpha / 255);
                sx += 1;
            }
            if ar < 255 {
                self.px(ex, y + row, col, ar * alpha / 255);
                ex -= 1;
            }
            if ex >= sx {
                self.rect(sx, y + row, ex - sx + 1, 1, col, alpha);
            }
        }
    }

    /// Anti-aliased filled circle.
    pub fn circle(&mut self, cx: i32, cy: i32, r: i32, color: u32, alpha: u32) {
        let outer = (r + 1) * (r + 1);
        let inner = if r >= 1 { (r - 1) * (r - 1) } else { 0 };
        let span = (outer - inner).max(1);
        for y in (cy - r - 1)..=(cy + r + 1) {
            for x in (cx - r - 1)..=(cx + r + 1) {
                let (dx, dy) = (x - cx, y - cy);
                let d2 = dx * dx + dy * dy;
                if d2 <= inner {
                    self.px(x, y, color, alpha);
                } else if d2 < outer {
                    self.px(x, y, color, alpha * ((outer - d2) as u32) / span as u32);
                }
            }
        }
    }

    /// 1 px circle outline (midpoint algorithm).
    pub fn ring(&mut self, cx: i32, cy: i32, r: i32, color: u32, alpha: u32) {
        let mut x = r;
        let mut y = 0;
        let mut d = 1 - r;
        while x >= y {
            self.px(cx + x, cy + y, color, alpha);
            self.px(cx - x, cy + y, color, alpha);
            self.px(cx + x, cy - y, color, alpha);
            self.px(cx - x, cy - y, color, alpha);
            self.px(cx + y, cy + x, color, alpha);
            self.px(cx - y, cy + x, color, alpha);
            self.px(cx + y, cy - x, color, alpha);
            self.px(cx - y, cy - x, color, alpha);
            y += 1;
            if d < 0 {
                d += 2 * y + 1;
            } else {
                x -= 1;
                d += 2 * (y - x) + 1;
            }
        }
    }

    /// 1 px line (Bresenham).
    pub fn line(&mut self, mut x0: i32, mut y0: i32, x1: i32, y1: i32, color: u32, alpha: u32) {
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            self.px(x0, y0, color, alpha);
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                err += dx;
                y0 += sy;
            }
        }
    }

    /// Text with the 8x16 console font. `scale` enlarges it by an integer factor.
    pub fn text_ex(&mut self, mut x: i32, y: i32, s: &str, scale: i32, color: u32, alpha: u32) {
        let font = self.font;
        for ch in s.bytes() {
            let glyph = font[ch as usize];
            for (row, bits) in glyph.iter().enumerate() {
                if *bits == 0 {
                    continue;
                }
                for col in 0..8i32 {
                    if *bits & (0x80u8 >> col) != 0 {
                        if scale == 1 {
                            self.px(x + col, y + row as i32, color, alpha);
                        } else {
                            self.rect(x + col * scale, y + row as i32 * scale, scale, scale, color, alpha);
                        }
                    }
                }
            }
            x += 8 * scale;
        }
    }

    pub fn text(&mut self, x: i32, y: i32, s: &str, color: u32) {
        self.text_ex(x, y, s, 1, color, 255);
    }
}