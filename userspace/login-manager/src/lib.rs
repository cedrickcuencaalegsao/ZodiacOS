#![no_std]
extern crate alloc;

use alloc::vec::Vec;

pub mod authentication;
pub mod greeter;
pub mod session;
pub mod user_accounts;

use session::{AuthError, Session};
use user_accounts::UserInfo;

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Color {
    Black = 0,
    Blue,
    Green,
    Cyan,
    Red,
    Magenta,
    Brown,
    LightGray,
    DarkGray,
    LightBlue,
    LightGreen,
    LightCyan,
    LightRed,
    Pink,
    Yellow,
    White,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Backspace,
    Tab,
    Up,
    Down,
    Esc,
}

/// Everything the UI can ask of the kernel.
pub trait Platform {
    // ---- display ----
    fn size(&self) -> (usize, usize);
    fn put_char(&mut self, x: usize, y: usize, ch: char, fg: Color, bg: Color);
    fn clear(&mut self, bg: Color);
    // ---- input / time ----
    fn read_key(&mut self) -> Key; // blocking
    fn sleep_ms(&mut self, ms: u64);
    // ---- kernel account services (all logic lives in the kernel) ----
    fn has_accounts(&self) -> bool;
    fn list_users(&self) -> Vec<UserInfo>;
    fn authenticate(&mut self, name: &str, password: &str) -> Result<Session, AuthError>;
    fn create_root(&mut self, password: &str) -> Result<(), AuthError>;
    
    fn gfx(&mut self) -> Option<&mut dyn Gfx> {
        None
    }
}

/// Entry point the kernel calls instead of starting the terminal.
pub fn run<P: Platform>(p: &mut P) -> Session {
    greeter::show(p);
    let session = authentication::run(p);
    greeter::welcome_user(p, &session);
    session
}

// ---------- tiny drawing helpers shared by the screens ----------

pub fn text<P: Platform>(p: &mut P, x: usize, y: usize, s: &str, fg: Color, bg: Color) {
    for (i, c) in s.chars().enumerate() {
        p.put_char(x + i, y, c, fg, bg);
    }
}

pub fn centered<P: Platform>(
    p: &mut P,
    x: usize,
    w: usize,
    y: usize,
    s: &str,
    fg: Color,
    bg: Color,
) {
    let len = s.chars().count();
    text(p, x + w.saturating_sub(len) / 2, y, s, fg, bg);
}

pub fn fill<P: Platform>(p: &mut P, x: usize, y: usize, w: usize, h: usize, bg: Color) {
    for yy in y..y + h {
        for xx in x..x + w {
            p.put_char(xx, yy, ' ', Color::White, bg);
        }
    }
}

pub fn border<P: Platform>(
    p: &mut P,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    fg: Color,
    bg: Color,
) {
    for xx in x + 1..x + w - 1 {
        p.put_char(xx, y, '-', fg, bg);
        p.put_char(xx, y + h - 1, '-', fg, bg);
    }
    for yy in y + 1..y + h - 1 {
        p.put_char(x, yy, '|', fg, bg);
        p.put_char(x + w - 1, yy, '|', fg, bg);
    }
    for (cx, cy) in [
        (x, y),
        (x + w - 1, y),
        (x, y + h - 1),
        (x + w - 1, y + h - 1),
    ] {
        p.put_char(cx, cy, '+', fg, bg);
    }
}

pub trait Gfx {
    fn size_px(&self) -> (usize, usize);
    fn gradient(&mut self, x: usize, y: usize, w: usize, h: usize, top: u32, bottom: u32);
    fn round_rect(&mut self, x: usize, y: usize, w: usize, h: usize, r: usize, rgb: u32, alpha: u8);
    fn text(&mut self, x: usize, y: usize, s: &str, scale: usize, rgb: u32);
    fn text_width(&self, s: &str, scale: usize) -> usize;
}
