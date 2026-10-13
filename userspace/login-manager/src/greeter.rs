use crate::session::Session;
use crate::{centered, Color, Gfx, Platform};
use alloc::format;

// Minimal "monochrome on black" look, matching the splash screen:
// grayscale only, a softly glowing star, hairlines, spaced lettering, fades.

const FADE_STEPS: u32 = 12;
const FADE_MS: u64 = 22; // sleep_ms takes u64

/// Total time the boot welcome screen is shown (fade in + hold + fade out).
const WELCOME_MS: u64 = 5000;

// ---- Helpers ---------------------------------------------------------------------

/// Gray level `v` (0..=255) at brightness `b` (0..=256) as 0xRRGGBB.
fn gray(v: u32, b: u32) -> u32 {
    (v * b / 256).min(255) * 0x01_01_01
}

/// Alpha `a` (0..=255) scaled by brightness `b` (0..=256).
fn alpha(a: u32, b: u32) -> u8 {
    (a * b / 256).min(255) as u8
}

/// Smoothstep: eases 0..=256 -> 0..=256.
fn ease(x: u32) -> u32 {
    (3 * x * x * 256 - 2 * x * x * x) / 65536
}

fn black(g: &mut dyn Gfx, x: usize, y: usize, w: usize, h: usize) {
    g.gradient(x, y, w, h, 0x000000, 0x000000);
}

/// Clear the whole screen to black. (Reads the size first so `g` isn't
/// borrowed twice in one call expression.)
fn black_all(g: &mut dyn Gfx) {
    let (w, h) = g.size_px();
    black(g, 0, 0, w, h);
}

/// Letter-spaced text, centred on `cx`. `gap` is extra pixels between letters.
fn spaced(g: &mut dyn Gfx, cx: usize, y: usize, s: &str, scale: usize, gap: usize, rgb: u32) {
    let n = s.len();
    if n == 0 {
        return;
    }
    let cw = g.text_width("M", scale);
    let total = n * cw + (n - 1) * gap;
    let mut x = cx.saturating_sub(total / 2);
    for b in s.bytes() {
        let buf = [b];
        if let Ok(ch) = core::str::from_utf8(&buf) {
            g.text(x, y, ch, scale, rgb);
        }
        x += cw + gap;
    }
}

/// A small white star with a soft halo (same idea as the splash stars).
fn star(g: &mut dyn Gfx, cx: usize, cy: usize, b: u32) {
    for i in (1..=6usize).rev() {
        let r = 4 + i * 4;
        g.round_rect(
            cx.saturating_sub(r),
            cy.saturating_sub(r),
            2 * r,
            2 * r,
            r,
            0xFFFFFF,
            alpha(7, b),
        );
    }
    g.round_rect(cx.saturating_sub(4), cy.saturating_sub(4), 8, 8, 4, gray(255, b), 255);
}

fn hairline(g: &mut dyn Gfx, cx: usize, y: usize, len: usize, level: u32, b: u32) {
    g.round_rect(cx.saturating_sub(len / 2), y, len, 2, 1, gray(level, b), 255);
}

/// Draw a frame at brightness `b` (0..=256), then sleep.
fn fade<P: Platform>(p: &mut P, up: bool, f: impl Fn(&mut dyn Gfx, u32)) {
    for i in 0..=FADE_STEPS {
        let k = if up { i } else { FADE_STEPS - i };
        let b = ease(k * 256 / FADE_STEPS);
        if let Some(g) = p.gfx() {
            f(g, b);
        }
        p.sleep_ms(FADE_MS);
    }
}

// ---- HD path ---------------------------------------------------------------------

fn hd_welcome(g: &mut dyn Gfx, b: u32) {
    let (sw, sh) = g.size_px();
    let (cx, cy) = (sw / 2, sh / 2);

    black(g, cx.saturating_sub(320), cy.saturating_sub(120), 640, 220);
    star(g, cx, cy - 84, b);
    spaced(g, cx, cy - 24, "ZODIACOS", 3, 18, gray(0xFF, b));
    hairline(g, cx, cy + 44, 120, 0x50, b);
    spaced(g, cx, cy + 64, "version 0.1.0", 1, 5, gray(0x70, b));
}

fn hd_user(g: &mut dyn Gfx, name: &str, b: u32) {
    let (sw, sh) = g.size_px();
    let (cx, cy) = (sw / 2, sh / 2);

    black(g, cx.saturating_sub(320), cy.saturating_sub(120), 640, 220);
    star(g, cx, cy - 84, b);
    spaced(g, cx, cy - 36, "welcome", 1, 8, gray(0x70, b));
    spaced(g, cx, cy - 4, name, 3, 10, gray(0xFF, b));
    hairline(g, cx, cy + 64, 120, 0x50, b);
}

// ---- Text-mode fallback ------------------------------------------------------------

fn text_screen<P: Platform>(p: &mut P, lines: &[(isize, &str, Color)]) {
    let (sw, sh) = p.size();
    p.clear(Color::Black);
    for &(dy, s, c) in lines {
        let y = (sh as isize / 2 + dy).max(0) as usize;
        centered(p, 0, sw, y, s, c, Color::Black);
    }
}

// ---- Public screens ----------------------------------------------------------------

/// Boot welcome screen. Shows for `WELCOME_MS`, then returns so the caller
/// can continue straight to authentication.
pub fn show<P: Platform>(p: &mut P) {
    if p.gfx().is_some() {
        if let Some(g) = p.gfx() {
            black_all(g);
        }
        let fades = 2 * (FADE_STEPS as u64 + 1) * FADE_MS;
        fade(p, true, |g, b| hd_welcome(g, b));
        p.sleep_ms(WELCOME_MS.saturating_sub(fades));
        fade(p, false, |g, b| hd_welcome(g, b));
        return;
    }

    text_screen(
        p,
        &[
            (-1, "Z O D I A C O S", Color::LightGray),
            (1, "version 0.1.0", Color::DarkGray),
        ],
    );
    p.sleep_ms(WELCOME_MS);
}

/// Shown briefly after a successful login.
pub fn welcome_user<P: Platform>(p: &mut P, session: &Session) {
    let name = session.name.as_str();

    if p.gfx().is_some() {
        if let Some(g) = p.gfx() {
            black_all(g);
        }
        fade(p, true, |g, b| hd_user(g, name, b));
        p.sleep_ms(800);
        fade(p, false, |g, b| hd_user(g, name, b));
        return;
    }

    let line = format!("{}", name);
    text_screen(p, &[(-1, "welcome", Color::DarkGray), (1, &line, Color::LightGray)]);
    p.sleep_ms(1200);
}