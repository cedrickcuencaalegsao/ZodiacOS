//! Night-sky wallpaper: gradient, a zodiac wheel, stars and a constellation.
use crate::gfx::{blend, Gfx};

// (cos, sin) * 1000 for the 12 zodiac directions, starting at the top, going clockwise.
const DIRS: [(i32, i32); 12] = [
    (0, -1000),
    (500, -866),
    (866, -500),
    (1000, 0),
    (866, 500),
    (500, 866),
    (0, 1000),
    (-500, 866),
    (-866, 500),
    (-1000, 0),
    (-866, -500),
    (-500, -866),
];

// A constellation (Big Dipper-like), in per-mille of the screen size.
const STARS: [(i32, i32); 7] = [(110, 150), (190, 185), (270, 165), (340, 215), (355, 300), (450, 315), (480, 225)];
const LINKS: [(usize, usize); 7] = [(0, 1), (1, 2), (2, 3), (3, 4), (4, 5), (5, 6), (6, 3)];

pub fn draw(g: &mut Gfx) {
    // 1. sky gradient: deep navy -> violet -> magenta glow at the horizon
    let (top, mid, bot) = (0x0a0d2e_u32, 0x2b1659_u32, 0x8a3b8f_u32);
    for y in 0..g.h {
        let t = (y * 255 / (g.h - 1).max(1)) as u32;
        let c = if t < 140 {
            blend(top, mid, t * 255 / 140)
        } else {
            blend(mid, bot, (t - 140) * 255 / 115)
        };
        g.rect(0, y, g.w, 1, c, 255);
    }

    // 2. zodiac wheel
    let (cx, cy) = (g.w / 2, g.h / 2);
    let r = g.w.min(g.h) * 2 / 5;
    let r2 = r - 16;
    let r3 = r * 55 / 100;
    let tint = 0xB9A6FF_u32;
    g.ring(cx, cy, r, tint, 70);
    g.ring(cx, cy, r + 1, tint, 28);
    g.ring(cx, cy, r2, tint, 60);
    g.ring(cx, cy, r3, tint, 40);
    for &(c, s) in DIRS.iter() {
        let p = |rad: i32| (cx + c * rad / 1000, cy + s * rad / 1000);
        let (x0, y0) = p(r3);
        let (x1, y1) = p(r2);
        g.line(x0, y0, x1, y1, tint, 26);
        let (x2, y2) = p(r);
        g.line(x1, y1, x2, y2, tint, 70);
        let (dx, dy) = p((r + r2) / 2);
        g.circle(dx, dy, 3, 0xE9E0FF, 190);
    }

    // 3. stars (deterministic pseudo-random)
    let (w, h) = (g.w.max(1) as u32, g.h.max(1) as u32);
    let mut seed: u32 = 0x2545_F491;
    for i in 0..160 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let x = ((seed >> 8) % w) as i32;
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let y = ((seed >> 8) % h) as i32;
        let a = 60 + ((seed >> 24) % 150);
        g.px(x, y, 0xFFFFFF, a);
        if i % 7 == 0 {
            g.px(x + 1, y, 0xFFFFFF, a / 2);
            g.px(x - 1, y, 0xFFFFFF, a / 2);
            g.px(x, y + 1, 0xFFFFFF, a / 2);
            g.px(x, y - 1, 0xFFFFFF, a / 2);
        }
    }

    // 4. constellation
    let mut pts = [(0i32, 0i32); 7];
    for (i, &(sx, sy)) in STARS.iter().enumerate() {
        pts[i] = (g.w * sx / 1000, g.h * sy / 1000);
    }
    for &(a, b) in LINKS.iter() {
        g.line(pts[a].0, pts[a].1, pts[b].0, pts[b].1, 0xCFC4FF, 60);
    }
    for &(x, y) in pts.iter() {
        g.circle(x, y, 7, 0xB9A6FF, 36);
        g.circle(x, y, 3, 0xFFFFFF, 235);
    }
}