use crate::arch::sleep_ms;
use crate::console::fbcon;
use crate::vga::{self, put_char};
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering::Relaxed};

// Splash screen driven by real boot progress.
//
//   splash::begin(total, done)  draw the logo and an empty bar (no sleeping)
//   splash::step()              one init stage finished, advance the bar
//   splash::finish()            fill to 100%, flash/fade out, clear the screen
//   splash::abort()             leave the splash immediately (init failed)
//   splash::active()            true while the splash owns the screen
//
// begin() and step() never call sleep_ms, because they run before the timer
// interrupt is enabled. Only finish() animates, so call it after
// interrupts::enable().

// Logo bitmap: 50 x 30 pixels. Each text cell holds two stacked pixels
// (half blocks), so at scale 1 it takes 50 columns x 15 rows.
const W: usize = 50;
const H: usize = 30; // must be even
const MAX_SCALE: usize = 4;

// Timing (ms), used by finish() only
const END_HOLD_MS: u32 = 300;
const FLASH_MS: u32 = 120;
const FADE_MS: u32 = 100;

// Palette (all on black)
const DIM: u8 = 0x08;
const SOFT: u8 = 0x07;
const WHITE: u8 = 0x0F;

// CP437 characters
const FULL: u8 = 0xDB;
const UPPER: u8 = 0xDF;
const LOWER: u8 = 0xDC;
const BAR_CH: u8 = 0xC4; // hairline; use LOWER for a thicker bar

/// Stars as (x, y, size): size 2 is a large star, size 1 a small one.
const STARS: [(i32, i32, i32); 9] = [
    (47, 8, 2),  // 0  right tip, large
    (43, 3, 1),  // 1
    (34, 8, 1),  // 2
    (33, 14, 2), // 3  junction, large
    (40, 18, 1), // 4
    (39, 26, 2), // 5  large
    (15, 22, 1), // 6
    (3, 24, 2),  // 7  left tip, large
    (13, 14, 1), // 8
];

/// Stars joined in order; the last hop closes the loop back to star 3.
const PATH: [usize; 10] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 3];

// ───────────── Progress state ─────────────

static ACTIVE: AtomicBool = AtomicBool::new(false);
static TOTAL: AtomicUsize = AtomicUsize::new(1);
static DONE: AtomicUsize = AtomicUsize::new(0);
static SHOWN: AtomicI64 = AtomicI64::new(0); // bar fill already on screen (pixels)

/// True while the splash owns the screen. Make the logger skip the screen
/// (serial only) while this is set, or log lines will draw over the logo.
pub fn active() -> bool {
    ACTIVE.load(Relaxed)
}

/// Start the splash. `total` is the number of init stages, `done` how many
/// have already finished (e.g. memory + drivers before the framebuffer exists).
pub fn begin(total: usize, done: usize) {
    TOTAL.store(total.max(1), Relaxed);
    DONE.store(done, Relaxed);
    SHOWN.store(0, Relaxed);
    ACTIVE.store(true, Relaxed);
    if fbcon::active() {
        hd_begin();
    } else {
        text_begin();
    }
    redraw_bar();
}

/// One init stage finished.
pub fn step() {
    if !active() {
        return;
    }
    DONE.fetch_add(1, Relaxed);
    redraw_bar();
}

/// Leave the splash right away (init failed, panic path).
pub fn abort() {
    if !ACTIVE.swap(false, Relaxed) {
        return;
    }
    vga::clear();
    vga::enable_cursor();
}

/// Keep the splash (with its current bar) on screen for `ms` milliseconds.
/// Needs the timer running, so call it after interrupts::enable().
pub fn hold(ms: u32) {
    if active() {
        sleep_ms(ms);
    }
}

// Optional: guarantee a minimum total splash time instead of a fixed delay.
// Needs an uptime counter from your timer; adapt `uptime_ms` to what you have.
//
// static START_MS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
// (in begin():  START_MS.store(crate::arch::uptime_ms(), Relaxed);)
//
// pub fn hold_until(min_ms: u32) {
//     let elapsed = crate::arch::uptime_ms().saturating_sub(START_MS.load(Relaxed));
//     if elapsed < min_ms as u64 {
//         sleep_ms(min_ms - elapsed as u32);
//     }
// }

/// Fill the bar, hold, fade out and hand the screen to the shell.
/// Needs the timer running (sleep_ms).
pub fn finish() {
    if !active() {
        return;
    }
    DONE.store(TOTAL.load(Relaxed), Relaxed);
    redraw_bar();
    sleep_ms(END_HOLD_MS);
    if fbcon::active() {
        hd_fade();
    } else {
        text_fade();
    }
    ACTIVE.store(false, Relaxed);
    vga::clear(); // resets the cursor to row 0 for the shell
    vga::enable_cursor();
}

fn redraw_bar() {
    let num = DONE.load(Relaxed).min(TOTAL.load(Relaxed)) as i64;
    let den = TOTAL.load(Relaxed) as i64;
    if fbcon::active() {
        let g = hd_layout();
        let fill = g.bar_len * num / den;
        let prev = SHOWN.swap(fill, Relaxed);
        let pad = BAR_HW / 16 + 2;
        bar_draw(g.bx, g.by, g.bar_len, BAR_HW, fill, g.bx + prev - pad - 2, g.bx + fill + pad + 2, 256);
    } else {
        let t = text_layout();
        let fill = t.bar_len as i64 * num / den;
        SHOWN.store(fill, Relaxed);
        draw_bar(t.bar_row, t.bar_col, t.bar_len, fill as usize, WHITE, DIM);
    }
}

// ───────────── Text-mode path (VGA text / fallback) ─────────────

struct Logo {
    lines: [u64; H],
    stars: [u64; H],
}

struct TextLayout {
    s: usize,
    top: usize,
    logo_col: usize,
    bar_len: usize,
    bar_col: usize,
    bar_row: usize,
}

fn center(width: usize, len: usize) -> usize {
    width.saturating_sub(len) / 2
}

fn plot(bm: &mut [u64; H], x: i32, y: i32) {
    if x >= 0 && (x as usize) < W && y >= 0 && (y as usize) < H {
        bm[y as usize] |= 1u64 << x;
    }
}

/// Bresenham line between two points.
fn line(bm: &mut [u64; H], (mut x0, mut y0): (i32, i32), (x1, y1): (i32, i32)) {
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        plot(bm, x0, y0);
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

fn add_star(bm: &mut [u64; H], (cx, cy, size): (i32, i32, i32)) {
    let (r, lim) = if size == 2 { (2, 5) } else { (1, 2) };
    for dy in -r..=r {
        for dx in -r..=r {
            if dx * dx + dy * dy <= lim {
                plot(bm, cx + dx, cy + dy);
            }
        }
    }
}

/// The complete constellation: every line and star from PATH.
fn build_logo() -> Logo {
    let mut logo = Logo { lines: [0; H], stars: [0; H] };
    add_star(&mut logo.stars, STARS[PATH[0]]);
    for pair in PATH.windows(2) {
        let (a, b) = (STARS[pair[0]], STARS[pair[1]]);
        line(&mut logo.lines, (a.0, a.1), (b.0, b.1));
        add_star(&mut logo.stars, b);
    }
    logo
}

/// Draw the logo at an integer scale. Each bitmap pixel becomes a `s` x `s`
/// block of half-cells, so pixels stay square on an 8x16 cell grid.
fn draw_logo(l: &Logo, row: usize, col: usize, s: usize, line_c: u8, star_c: u8) {
    for cy in 0..H * s / 2 {
        let (y0, y1) = (cy * 2 / s, (cy * 2 + 1) / s);
        for cx in 0..W * s {
            let x = cx / s;
            let bit = |bm: &[u64; H], y: usize| bm[y] >> x & 1 == 1;
            let (s0, s1) = (bit(&l.stars, y0), bit(&l.stars, y1));
            let top = s0 || bit(&l.lines, y0);
            let bottom = s1 || bit(&l.lines, y1);
            let ch = match (top, bottom) {
                (true, true) => FULL,
                (true, false) => UPPER,
                (false, true) => LOWER,
                _ => b' ',
            };
            put_char(row + cy, col + cx, ch, if s0 || s1 { star_c } else { line_c });
        }
    }
}

fn draw_bar(row: usize, col: usize, len: usize, filled: usize, fill_c: u8, track_c: u8) {
    for i in 0..len {
        put_char(row, col + i, BAR_CH, if i < filled { fill_c } else { track_c });
    }
}

/// Scale and positions for the text-mode splash (pure function of screen size).
fn text_layout() -> TextLayout {
    let (w, h) = (vga::width(), vga::height());

    // Largest scale that leaves a margin on every side.
    let mut s = 1;
    while s < MAX_SCALE && W * (s + 1) + 8 <= w && H * (s + 1) / 2 + (2 + (s + 1)) + 1 + 4 <= h {
        s += 1;
    }
    let rows = H * s / 2;
    let gap = 2 + s;
    let block_h = rows + gap + 1;
    // Centered, nudged up a little (optical centre).
    let top = (h.saturating_sub(block_h) / 2).saturating_sub(h / 16);
    let bar_len = (W * s * 3 / 5).max(10);
    TextLayout {
        s,
        top,
        logo_col: center(w, W * s),
        bar_len,
        bar_col: center(w, bar_len),
        bar_row: top + rows + gap,
    }
}

fn text_begin() {
    let (w, h) = (vga::width(), vga::height());
    let t = text_layout();

    vga::disable_cursor();
    for r in 0..h {
        for c in 0..w {
            put_char(r, c, b' ', SOFT);
        }
    }

    let logo = build_logo();
    draw_logo(&logo, t.top, t.logo_col, t.s, SOFT, WHITE);
    draw_bar(t.bar_row, t.bar_col, t.bar_len, 0, WHITE, DIM);
}

fn text_fade() {
    let t = text_layout();
    let logo = build_logo();

    // Flash everything white, then fade out to black.
    draw_logo(&logo, t.top, t.logo_col, t.s, WHITE, WHITE);
    sleep_ms(FLASH_MS);
    for c in [SOFT, DIM] {
        draw_logo(&logo, t.top, t.logo_col, t.s, c, c);
        draw_bar(t.bar_row, t.bar_col, t.bar_len, t.bar_len, c, c);
        sleep_ms(FADE_MS);
    }
}

// ───────────── HD path (framebuffer console) ─────────────
// Draws the constellation at pixel resolution with anti-aliasing and a soft
// halo around the stars. Integer math only, so it doesn't need SSE/floats.

const BUF_W: usize = 560; // logo canvas: enough for a scale of 11 (50*11 x 30*11)
const BUF_H: usize = 340;
const MAX_K: i64 = 11;
const LINE_I: i64 = 170; // line brightness (0..255)
const GLOW_I: i64 = 40; // star halo brightness at the star's edge, 0 disables it
const TRACK_I: i64 = 56; // progress-bar track brightness
const BAR_HW: i64 = 48; // bar half-thickness in 1/16 px (3 px)

struct Canvas(UnsafeCell<[u8; BUF_W * BUF_H]>);
unsafe impl Sync for Canvas {}
static CANVAS: Canvas = Canvas(UnsafeCell::new([0; BUF_W * BUF_H]));

fn canvas() -> &'static mut [u8; BUF_W * BUF_H] {
    unsafe { &mut *CANVAS.0.get() }
}

#[derive(Clone, Copy)]
struct Rect {
    x0: i64,
    y0: i64,
    x1: i64, // exclusive
    y1: i64, // exclusive
}

/// Screen geometry of the HD splash (pure function of the screen size).
struct Hd {
    k: i64,       // pixels per logo unit
    ox: i64,      // logo origin on screen
    oy: i64,
    bx: i64,      // progress bar start
    by: i64,
    bar_len: i64,
    w: i64,
    h: i64,
}

fn hd_layout() -> Hd {
    let (pw, ph) = fbcon::pixel_size();
    let (w, h) = (pw as i64, ph as i64);

    // Pixels per logo unit: the logo takes about 28% of the width / 40% of the height.
    let k = (w * 28 / 100 / W as i64).min(h * 40 / 100 / H as i64).clamp(1, MAX_K);
    let (lw, lh) = (W as i64 * k, H as i64 * k);
    let bar_len = lw * 3 / 5;

    Hd {
        k,
        ox: (w - lw) / 2, // logo exactly centered on the screen
        oy: (h - lh) / 2,
        bx: (w - bar_len) / 2, // bar centered horizontally, 85% of the way down
        by: h * 85 / 100,
        bar_len,
        w,
        h,
    }
}

fn isqrt(n: i64) -> i64 {
    if n < 2 {
        return n;
    }
    let mut x = 1i64 << ((64 - n.leading_zeros() as i64 + 1) / 2); // >= sqrt(n)
    loop {
        let y = (x + n / x) / 2;
        if y >= x {
            return x;
        }
        x = y;
    }
}

/// Coverage 0..=255 for a pixel `d16` away (1/16 px units) from a shape edge at `r16`.
fn cov(d16: i64, r16: i64) -> i64 {
    ((r16 + 8 - d16) * 255 / 16).clamp(0, 255)
}

/// Distance from pixel (px, py) to the segment a-b, in 1/16 px.
fn seg_dist16(px: i64, py: i64, a: (i64, i64), b: (i64, i64)) -> i64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let (apx, apy) = (px - a.0, py - a.1);
    let len2 = dx * dx + dy * dy;
    let t = apx * dx + apy * dy;
    let d2 = if t <= 0 || len2 == 0 {
        apx * apx + apy * apy
    } else if t >= len2 {
        let (bpx, bpy) = (px - b.0, py - b.1);
        bpx * bpx + bpy * bpy
    } else {
        let cr = apx * dy - apy * dx;
        return isqrt(cr * cr * 256 / len2);
    };
    isqrt(d2 * 256)
}

fn put_max(x: i64, y: i64, v: i64) {
    if v > 0 && x >= 0 && y >= 0 && (x as usize) < BUF_W && (y as usize) < BUF_H {
        let p = &mut canvas()[y as usize * BUF_W + x as usize];
        if v as u8 > *p {
            *p = v as u8;
        }
    }
}

/// Anti-aliased round-capped line into the canvas. `hw16` = half width in 1/16 px.
fn stroke(a: (i64, i64), b: (i64, i64), hw16: i64, level: i64) -> Rect {
    let pad = hw16 / 16 + 3;
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let horizontal = dx.abs() >= dy.abs();
    let (lo, hi) = if horizontal {
        (a.0.min(b.0), a.0.max(b.0))
    } else {
        (a.1.min(b.1), a.1.max(b.1))
    };
    // Walk along the major axis and only test a thin band around the line.
    for m in lo - pad..=hi + pad {
        let c = if horizontal {
            if dx == 0 { a.1 } else { a.1 + (m - a.0) * dy / dx }
        } else {
            a.0 + (m - a.1) * dx / dy
        };
        for n in c - pad - 1..=c + pad + 1 {
            let (x, y) = if horizontal { (m, n) } else { (n, m) };
            put_max(x, y, cov(seg_dist16(x, y, a, b), hw16) * level / 255);
        }
    }
    Rect {
        x0: a.0.min(b.0) - pad - 1,
        y0: a.1.min(b.1) - pad - 1,
        x1: a.0.max(b.0) + pad + 2,
        y1: a.1.max(b.1) + pad + 2,
    }
}

/// Anti-aliased filled circle with a soft halo.
fn star(c: (i64, i64), r16: i64) -> Rect {
    let halo = r16 * 5 / 2;
    let reach = halo / 16 + 2;
    for y in c.1 - reach..=c.1 + reach {
        for x in c.0 - reach..=c.0 + reach {
            let (dx, dy) = (x - c.0, y - c.1);
            let d16 = isqrt((dx * dx + dy * dy) * 256);
            let mut v = cov(d16, r16);
            if GLOW_I > 0 && v < 255 && d16 < halo {
                let t = (halo - d16) * 256 / (halo - r16); // ~256 at the edge, 0 at the halo's end
                v = v.max(t * t * GLOW_I / 65536);
            }
            put_max(x, y, v);
        }
    }
    Rect { x0: c.0 - reach, y0: c.1 - reach, x1: c.0 + reach + 1, y1: c.1 + reach + 1 }
}

/// Push a canvas region to the screen at brightness `bright` (0..=256).
fn flush(ox: usize, oy: usize, r: Rect, bright: i64) {
    let (x0, y0) = (r.x0.max(0), r.y0.max(0));
    let (x1, y1) = (r.x1.min(BUF_W as i64), r.y1.min(BUF_H as i64));
    let buf = canvas();
    for y in y0..y1 {
        let row = &buf[y as usize * BUF_W..(y as usize + 1) * BUF_W];
        let mut x = x0;
        while x < x1 {
            let v = row[x as usize] as i64 * bright / 256;
            let mut e = x + 1;
            while e < x1 && row[e as usize] as i64 * bright / 256 == v {
                e += 1; // merge equal pixels into one fill
            }
            if v > 0 || bright < 256 {
                fbcon::fill_gray(ox + x as usize, oy + y as usize, (e - x) as usize, 1, v as u8);
            }
            x = e;
        }
    }
}

/// Smooth capsule progress bar: dim track, white fill. Redraws columns c0..c1 only.
fn bar_draw(bx: i64, by: i64, len: i64, hw16: i64, fill: i64, c0: i64, c1: i64, bright: i64) {
    let pad = hw16 / 16 + 2;
    let (a, end, f_end) = ((bx, by), (bx + len, by), (bx + fill, by));
    for x in c0.max(bx - pad)..c1.min(bx + len + pad + 1) {
        for y in by - pad..=by + pad {
            let track = cov(seg_dist16(x, y, a, end), hw16) * TRACK_I / 255;
            let filled = if fill > 0 { cov(seg_dist16(x, y, a, f_end), hw16) } else { 0 };
            let v = track.max(filled) * bright / 256;
            fbcon::fill_gray(x as usize, y as usize, 1, 1, v as u8);
        }
    }
}

/// Black screen, full constellation drawn at once (no sleeping: the timer
/// isn't running yet), empty bar track.
fn hd_begin() {
    let g = hd_layout();
    let (ox, oy) = (g.ox as usize, g.oy as usize);

    vga::disable_cursor();
    fbcon::fill_gray(0, 0, g.w as usize, g.h as usize, 0);
    canvas().fill(0);

    let k = g.k;
    let pt = |i: usize| (STARS[i].0 as i64 * k + k / 2, STARS[i].1 as i64 * k + k / 2);
    let rad = |i: usize| if STARS[i].2 == 2 { 16 * k } else { 10 * k };
    let line_hw = (2 * k).max(12); // line half-width, 1/16 px

    flush(ox, oy, star(pt(PATH[0]), rad(PATH[0])), 256);
    for pair in PATH.windows(2) {
        flush(ox, oy, stroke(pt(pair[0]), pt(pair[1]), line_hw, LINE_I), 256);
        flush(ox, oy, star(pt(pair[1]), rad(pair[1])), 256);
    }

    bar_draw(g.bx, g.by, g.bar_len, BAR_HW, 0, 0, g.w, 256);
}

/// True fade to black.
fn hd_fade() {
    let g = hd_layout();
    let all = Rect { x0: 0, y0: 0, x1: BUF_W as i64, y1: BUF_H as i64 };
    for b in (0..8i64).rev() {
        flush(g.ox as usize, g.oy as usize, all, b * 32);
        bar_draw(g.bx, g.by, g.bar_len, BAR_HW, g.bar_len, 0, g.w, b * 32);
        sleep_ms(25);
    }
}