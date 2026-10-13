use crate::{fill, text, Color, Platform};

pub struct Button {
    pub x: usize,
    pub y: usize,
    pub label: &'static str,
    pub pressed: bool,
    pub width: usize,
}

impl Button {
    pub fn new(x: usize, y: usize, label: &'static str) -> Self {
        Self { x, y, label, pressed: false, width: 16 }
    }

    pub fn with_width(mut self, width: usize) -> Self {
        self.width = width;
        self
    }

    pub fn draw<P: Platform>(&self, p: &mut P) {
        let (fg, bg) = if self.pressed {
            (Color::Black, Color::Cyan)
        } else {
            (Color::White, Color::Blue)
        };
        fill(p, self.x, self.y, self.width, 1, bg);
        let len = self.label.chars().count();
        let lx = self.x + self.width.saturating_sub(len) / 2;
        text(p, lx, self.y, self.label, fg, bg);
    }
}