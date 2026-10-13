use crate::{border, fill, text, Color, Platform};
use alloc::string::String;

pub struct Input {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub value: String,
    pub secret: bool,
    pub focused: bool,
}

impl Input {
    pub fn new(x: usize, y: usize, width: usize, secret: bool) -> Self {
        Self { x, y, width, value: String::new(), secret, focused: true }
    }

    pub fn draw<P: Platform>(&self, p: &mut P) {
        let edge = if self.focused { Color::Cyan } else { Color::LightGray };
        border(p, self.x, self.y, self.width, 3, edge, Color::DarkGray);
        fill(p, self.x + 1, self.y + 1, self.width - 2, 1, Color::Black);

        let shown = self.value.chars().count().min(self.width - 4);
        let mut display = String::new();
        if self.secret {
            for _ in 0..shown {
                display.push('•');
            }
        } else {
            display = self.value.chars().take(self.width - 4).collect();
        }
        if self.focused {
            display.push('▌');
        }
        text(p, self.x + 1, self.y + 1, &display, Color::White, Color::Black);
    }

    pub fn push_char(&mut self, c: char) {
        if self.value.len() < 64 {
            self.value.push(c);
        }
    }
    pub fn backspace(&mut self) { self.value.pop(); }
    pub fn clear(&mut self) { self.value.clear(); }
    pub fn as_str(&self) -> &str { &self.value }
}