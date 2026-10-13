//! Top bar (logo, menu, focused title, clock) and the Zodiac drop-down menu.
use crate::gfx::{text_width, Gfx};
use core::fmt::Write;

pub const HEIGHT: i32 = 30;
const BTN_W: i32 = 128;

pub const MENU_ITEMS: [&str; 3] = ["About ZodiacOS", "Settings", "Log out"];
const MENU_X: i32 = 8;
const MENU_W: i32 = 190;
const ITEM_H: i32 = 30;
const MENU_PAD: i32 = 6;

struct Buf {
    b: [u8; 16],
    n: usize,
}
impl Write for Buf {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &c in s.as_bytes() {
            if self.n < self.b.len() {
                self.b[self.n] = c;
                self.n += 1;
            }
        }
        Ok(())
    }
}

pub fn draw(g: &mut Gfx, secs: u64, menu_open: bool, title: &str) {
    g.rect(0, 0, g.w, HEIGHT, 0x090916, 205);
    g.rect(0, HEIGHT - 1, g.w, 1, 0xFFFFFF, 22);

    if menu_open {
        g.round_rect(6, 3, BTN_W - 8, HEIGHT - 6, 7, 0xFFFFFF, 0xFFFFFF, 40);
    }
    // logo: ring with a dot
    g.ring(20, HEIGHT / 2, 7, 0xFFFFFF, 235);
    g.circle(20, HEIGHT / 2, 3, 0xB9A6FF, 255);
    g.text(36, 7, "ZodiacOS", 0xFFFFFF);

    if !title.is_empty() {
        let tw = text_width(title, 1);
        g.text((g.w - tw) / 2, 7, title, 0xC8C8E0);
    }

    // clock (uptime for now: swap for the RTC later)
    let mut buf = Buf { b: [0; 16], n: 0 };
    let _ = write!(buf, "{:02}:{:02}:{:02}", secs / 3600, (secs / 60) % 60, secs % 60);
    let s = core::str::from_utf8(&buf.b[..buf.n]).unwrap_or("");
    g.text(g.w - text_width(s, 1) - 14, 7, s, 0xE6E6F5);
}

pub fn menu_button_hit(x: i32, y: i32) -> bool {
    x >= 0 && x < BTN_W && y >= 0 && y < HEIGHT
}

fn menu_top() -> i32 {
    HEIGHT + 4
}

fn menu_h() -> i32 {
    MENU_ITEMS.len() as i32 * ITEM_H + 2 * MENU_PAD
}

pub fn menu_item_at(x: i32, y: i32) -> Option<usize> {
    let y0 = menu_top() + MENU_PAD;
    if x < MENU_X || x >= MENU_X + MENU_W || y < y0 || y >= y0 + MENU_ITEMS.len() as i32 * ITEM_H {
        return None;
    }
    Some(((y - y0) / ITEM_H) as usize)
}

pub fn draw_menu(g: &mut Gfx, hover: Option<usize>) {
    let (top, h) = (menu_top(), menu_h());
    g.round_rect(MENU_X - 1, top - 1, MENU_W + 2, h + 2, 11, 0xFFFFFF, 0xFFFFFF, 40);
    g.round_rect(MENU_X, top, MENU_W, h, 10, 0x1a1a36, 0x12122a, 245);
    for (i, name) in MENU_ITEMS.iter().enumerate() {
        let iy = top + MENU_PAD + i as i32 * ITEM_H;
        if hover == Some(i) {
            g.round_rect(MENU_X + 6, iy + 1, MENU_W - 12, ITEM_H - 2, 6, 0x6a4bd8, 0x5b3fa0, 255);
        }
        let color = if i == MENU_ITEMS.len() - 1 { 0xFF9A9A } else { 0xFFFFFF };
        g.text(MENU_X + 18, iy + 7, name, color);
    }
}