//! ZodiacOS desktop. A `no_std` library like `login-manager`: the kernel links it and
//! calls `run`. The desktop never touches hardware; the kernel provides a `Host`.
#![no_std]

mod apps;
mod desktop;
mod dock;
mod gfx;
mod panel;
mod wallpaper;

pub use desktop::run;

/// 256 glyphs, 16 rows each, one byte per row (bit 7 = leftmost pixel): the VGA 8x16 font.
pub type Font = [[u8; 16]; 256];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(u8),
    Up,
    Down,
    Left,
    Right,
    Enter,
    Escape,
}

#[derive(Clone, Copy)]
pub enum Event {
    /// Relative pointer movement in pixels (positive y = down).
    MouseMove { dx: i32, dy: i32 },
    /// button: 0 = left, 1 = right, 2 = middle
    Button { button: u8, down: bool },
    Key { key: Key, down: bool },
}

/// Everything the desktop needs from the kernel.
pub trait Host {
    /// Screen size in pixels.
    fn size(&self) -> (usize, usize);
    /// Next input event, without blocking.
    fn poll_event(&mut self) -> Option<Event>;
    fn uptime_ms(&mut self) -> u64;
    fn sleep_ms(&mut self, ms: u64);
    /// Copy a finished frame (`width * height` pixels, 0x00RRGGBB, row-major) to the screen.
    fn present(&mut self, pixels: &[u32]);
    /// The console font used for all text.
    fn font(&self) -> &'static Font;
}