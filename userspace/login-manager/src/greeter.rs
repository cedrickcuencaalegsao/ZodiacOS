use crate::session::Session;
use crate::{border, centered, fill, Color, Key, Platform};
use alloc::format;

fn frame<P: Platform>(p: &mut P, w: usize, h: usize) -> (usize, usize) {
    let (sw, sh) = p.size();
    let (x, y) = (sw.saturating_sub(w) / 2, sh.saturating_sub(h) / 2);
    p.clear(Color::Blue);
    fill(p, x, y, w, h, Color::LightGray);
    border(p, x, y, w, h, Color::Black, Color::LightGray);
    (x, y)
}

/// Boot welcome screen. Returns when the user presses Enter.
pub fn show<P: Platform>(p: &mut P) {
    let (w, h) = (44, 9);
    let (x, y) = frame(p, w, h);
    centered(p, x, w, y + 2, "Welcome to ZodiacOS", Color::Black, Color::LightGray);
    centered(p, x, w, y + 3, "version 0.1.0", Color::DarkGray, Color::LightGray);
    centered(p, x, w, y + 6, "[ Press Enter to continue ]", Color::Black, Color::LightGray);

    loop {
        if p.read_key() == Key::Enter {
            return;
        }
    }
}

/// Shown briefly after a successful login.
pub fn welcome_user<P: Platform>(p: &mut P, session: &Session) {
    let (w, h) = (44, 7);
    let (x, y) = frame(p, w, h);
    let line = format!("Welcome, {}!", session.name);
    centered(p, x, w, y + 3, &line, Color::Black, Color::LightGray);
    p.sleep_ms(1200);
}