//! The dock at the bottom of the screen.
use crate::apps::App;
use crate::gfx::{text_width, Gfx, TL, TR};

pub const APPS: [App; 3] = [App::Terminal, App::Files, App::Settings];

const ICON: i32 = 52;
const GAP: i32 = 14;
const PAD: i32 = 14;
const BOTTOM: i32 = 12;

/// (x, y, w, h) of the dock background.
fn bounds(sw: i32, sh: i32) -> (i32, i32, i32, i32) {
    let n = APPS.len() as i32;
    let w = n * ICON + (n - 1) * GAP + 2 * PAD;
    let h = ICON + 2 * PAD;
    ((sw - w) / 2, sh - h - BOTTOM, w, h)
}

fn icon_x(i: usize, x: i32) -> i32 {
    x + PAD + i as i32 * (ICON + GAP)
}

pub fn hit(mx: i32, my: i32, sw: i32, sh: i32) -> Option<usize> {
    let (x, y, _, h) = bounds(sw, sh);
    if my < y || my >= y + h {
        return None;
    }
    for i in 0..APPS.len() {
        let ix = icon_x(i, x);
        if mx >= ix - GAP / 2 && mx < ix + ICON + GAP / 2 {
            return Some(i);
        }
    }
    None
}

pub fn draw(g: &mut Gfx, open: [bool; 3], hover: Option<usize>) {
    let (x, y, w, h) = bounds(g.w, g.h);
    g.round_rect(x - 1, y - 1, w + 2, h + 2, 22, 0xFFFFFF, 0xFFFFFF, 36);
    g.round_rect(x, y, w, h, 21, 0x1a1a38, 0x101024, 215);

    for (i, app) in APPS.iter().enumerate() {
        let ix = icon_x(i, x);
        let mut iy = y + PAD;
        let hovered = hover == Some(i);
        if hovered {
            iy -= 6;
        }
        let (c0, c1) = app.colors();
        g.round_rect(ix, iy, ICON, ICON, 13, c0, c1, 255);
        g.round_rect_c(ix + 2, iy + 2, ICON - 4, ICON / 2 - 2, 11, TL | TR, 0xFFFFFF, 0xFFFFFF, 34);
        if hovered {
            g.round_rect(ix, iy, ICON, ICON, 13, 0xFFFFFF, 0xFFFFFF, 28);
        }

        let label = app.icon_label();
        let color = if *app == App::Terminal { 0x6CFF9C } else { 0xFFFFFF };
        let lx = ix + (ICON - text_width(label, 2)) / 2;
        g.text_ex(lx, iy + (ICON - 32) / 2, label, 2, color, 255);

        if open[i] {
            g.circle(ix + ICON / 2, y + h - 8, 2, 0xFFFFFF, 230);
        }
        if hovered {
            let name = app.title();
            let tw = text_width(name, 1);
            let tx = ix + ICON / 2 - tw / 2;
            g.round_rect(tx - 9, iy - 32, tw + 18, 24, 7, 0x000000, 0x000000, 210);
            g.text(tx, iy - 28, name, 0xFFFFFF);
        }
    }
}