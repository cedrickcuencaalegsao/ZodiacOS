use crate::{border, fill, text, Color, Platform};
use alloc::format;   // ← add this line

pub struct Panel {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

impl Panel {
    pub fn new(x: usize, y: usize, w: usize, h: usize) -> Self {
        Self { x, y, w, h }
    }

    pub fn draw<P: Platform>(&self, p: &mut P) {
        fill(p, self.x, self.y, self.w, self.h, Color::DarkGray);
        border(p, self.x, self.y, self.w, self.h, Color::LightGray, Color::DarkGray);
    }

    pub fn title<P: Platform>(&self, p: &mut P, title: &str) {
        text(
            p,
            self.x + 2,
            self.y,
            &format!(" {} ", title),
            Color::Cyan,
            Color::DarkGray,
        );
    }
}