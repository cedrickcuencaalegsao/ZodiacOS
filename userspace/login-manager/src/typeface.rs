//! Roboto text rendering for the login GUI (fontdue, runtime rasterisation + cache).
//!
//! Fonts are embedded in the binary. Use the ASCII-only subsets (see the
//! pyftsubset commands) so the kernel stays small.
//!
//! If a font fails to parse, `Ui` silently falls back to the built-in VGA bitmap font.

use crate::Gfx;
use alloc::{collections::BTreeMap, vec::Vec};
use fontdue::{Font, FontSettings};

// Paths are relative to THIS file (userspace/login-manager/src/typeface.rs).
// Use the STATIC fonts, not the VariableFont ones (fontdue can't pick variable instances).
static REGULAR: &[u8] = include_bytes!("../../../assets/fonts/Roboto/static/Roboto-Regular.ttf");
// One embedded font keeps the kernel under the bootloader's 512 KB limit.
// To get real bold back later (costs ~150 KB), use instead:
//   static BOLD: &[u8] = include_bytes!("../../../assets/fonts/Roboto/static/Roboto-Bold.ttf");
static BOLD: &[u8] = REGULAR;

struct Glyph {
    w: usize,
    h: usize,
    xmin: i32,
    ymin: i32,
    adv: i32,
    mask: Vec<u8>, // w*h coverage, 0..=255, top row first
}

struct Face {
    font: Font,
    cache: BTreeMap<(char, u16), Glyph>,
}

impl Face {
    fn new(data: &'static [u8]) -> Option<Face> {
        let font = Font::from_bytes(data, FontSettings::default()).ok()?;
        Some(Face { font, cache: BTreeMap::new() })
    }

    fn glyph(&mut self, ch: char, px: u16) -> &Glyph {
        let font = &self.font;
        self.cache.entry((ch, px)).or_insert_with(|| {
            let (m, mask) = font.rasterize(ch, px as f32);
            Glyph {
                w: m.width,
                h: m.height,
                xmin: m.xmin,
                ymin: m.ymin,
                adv: (m.advance_width + 0.5) as i32,
                mask,
            }
        })
    }

    fn width(&mut self, s: &str, px: u16) -> usize {
        let mut w = 0i32;
        for ch in s.chars() {
            w += self.glyph(ch, px).adv;
        }
        w.max(0) as usize
    }

    /// Draws `s` with its baseline at `baseline`, starting at pixel column `x`.
    fn draw(&mut self, c: &mut dyn Gfx, x: usize, baseline: usize, s: &str, px: u16, rgb: u32) {
        let mut pen = x as i32;
        for ch in s.chars() {
            let g = self.glyph(ch, px);
            let gx = pen + g.xmin;
            let gy = baseline as i32 - g.h as i32 - g.ymin;
            if g.w > 0 && gx >= 0 && gy >= 0 {
                c.blit_mask(gx as usize, gy as usize, g.w, g.h, &g.mask, rgb);
            }
            pen += g.adv;
        }
    }
}

/// Bitmap-font scale to use when Roboto isn't available.
fn fallback_scale(px: u16) -> usize {
    (px as usize / 16).max(1)
}

/// Regular + bold faces. All vertical positions are the MIDDLE of the capital
/// letters, which makes centring text inside buttons and fields trivial.
pub struct Ui {
    regular: Option<Face>,
    bold: Option<Face>,
}

impl Ui {
    pub fn new() -> Self {
        Ui { regular: Face::new(REGULAR), bold: Face::new(BOLD) }
    }

    fn face(&mut self, bold: bool) -> Option<&mut Face> {
        if bold { self.bold.as_mut() } else { self.regular.as_mut() }
    }

    pub fn width(&mut self, c: &mut dyn Gfx, s: &str, px: u16, bold: bool) -> usize {
        match self.face(bold) {
            Some(f) => f.width(s, px),
            None => c.text_width(s, fallback_scale(px)),
        }
    }

    /// Left-aligned text; `mid_y` is the vertical middle of capital letters.
    pub fn text(
        &mut self,
        c: &mut dyn Gfx,
        x: usize,
        mid_y: usize,
        s: &str,
        px: u16,
        bold: bool,
        rgb: u32,
    ) {
        match self.face(bold) {
            Some(f) => {
                let cap_height = px as usize * 711 / 1000; // Roboto cap height = 0.711 em
                f.draw(c, x, mid_y + cap_height / 2, s, px, rgb);
            }
            None => {
                let sc = fallback_scale(px);
                c.text(x, mid_y.saturating_sub(8 * sc), s, sc, rgb);
            }
        }
    }

    /// Horizontally centred on `cx`.
    pub fn centered(
        &mut self,
        c: &mut dyn Gfx,
        cx: usize,
        mid_y: usize,
        s: &str,
        px: u16,
        bold: bool,
        rgb: u32,
    ) {
        let w = self.width(c, s, px, bold);
        self.text(c, cx.saturating_sub(w / 2), mid_y, s, px, bold, rgb);
    }
}