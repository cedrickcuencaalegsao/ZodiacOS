//! The built-in apps and what they draw inside their windows.
use crate::gfx::{text_width, Gfx};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum App {
    Welcome,
    Terminal,
    Files,
    Settings,
}

impl App {
    pub fn title(self) -> &'static str {
        match self {
            App::Welcome => "Welcome",
            App::Terminal => "Terminal",
            App::Files => "Files",
            App::Settings => "Settings",
        }
    }

    pub fn size(self) -> (i32, i32) {
        match self {
            App::Welcome => (520, 320),
            App::Terminal => (560, 340),
            App::Files => (600, 380),
            App::Settings => (480, 340),
        }
    }

    /// Icon gradient (top, bottom).
    pub fn colors(self) -> (u32, u32) {
        match self {
            App::Welcome => (0x9a6bff, 0x5b3fa0),
            App::Terminal => (0x3a3a4e, 0x15151f),
            App::Files => (0x5cb3ff, 0x2a62d6),
            App::Settings => (0xa7adc4, 0x5a6078),
        }
    }

    pub fn icon_label(self) -> &'static str {
        match self {
            App::Welcome => "Z",
            App::Terminal => ">_",
            App::Files => "F",
            App::Settings => "S",
        }
    }
}

/// Draws the app's content into the area below the title bar.
pub fn draw_content(g: &mut Gfx, app: App, x: i32, y: i32, w: i32, h: i32, secs: u64) {
    match app {
        App::Welcome => welcome(g, x, y, w),
        App::Terminal => terminal(g, x, y, w, h, secs),
        App::Files => files(g, x, y, w, h),
        App::Settings => settings(g, x, y, w),
    }
}

fn welcome(g: &mut Gfx, x: i32, y: i32, w: i32) {
    g.text_ex(x + 28, y + 26, "Welcome to ZodiacOS", 2, 0xFFFFFF, 255);
    g.rect(x + 28, y + 66, 60, 3, 0x9a6bff, 255);
    let lines = [
        "A hobby operating system written in Rust.",
        "",
        "Open apps from the dock at the bottom.",
        "Drag a window by its title bar.",
        "Use the ZodiacOS menu to log out.",
    ];
    for (i, l) in lines.iter().enumerate() {
        g.text(x + 28, y + 88 + i as i32 * 22, l, 0xC9C9E0);
    }
    let (cx, cy) = (x + w - 74, y + 80);
    g.ring(cx, cy, 38, 0x9a6bff, 200);
    g.ring(cx, cy, 39, 0x9a6bff, 80);
    g.circle(cx, cy, 13, 0xB9A6FF, 235);
}

fn terminal(g: &mut Gfx, x: i32, y: i32, w: i32, h: i32, secs: u64) {
    g.round_rect(x + 10, y + 4, w - 20, h - 14, 8, 0x0b0b12, 0x0b0b12, 255);
    let lines = ["ZodiacOS 0.1.0 (x86_64)", "Type 'help' for a list of commands.", ""];
    for (i, l) in lines.iter().enumerate() {
        g.text(x + 24, y + 16 + i as i32 * 18, l, 0x9a9ab8);
    }
    let py = y + 16 + 3 * 18;
    let prompt = "zodiac@os:~$ ";
    g.text(x + 24, py, prompt, 0x6CFF9C);
    if secs % 2 == 0 {
        g.rect(x + 24 + text_width(prompt, 1), py, 8, 16, 0x6CFF9C, 255);
    }
}

fn files(g: &mut Gfx, x: i32, y: i32, w: i32, h: i32) {
    let _ = w;
    g.round_rect(x + 10, y + 4, 150, h - 14, 8, 0xFFFFFF, 0xFFFFFF, 12);
    let places = ["Home", "Documents", "Downloads", "Pictures", "Music"];
    for (i, p) in places.iter().enumerate() {
        let py = y + 12 + i as i32 * 32;
        if i == 0 {
            g.round_rect(x + 16, py, 138, 26, 6, 0x6a4bd8, 0x5b3fa0, 255);
        }
        g.text(x + 30, py + 5, p, 0xF0F0FA);
    }
    let items = [
        ("kernel", true),
        ("userspace", true),
        ("docs", true),
        ("build", true),
        ("README.md", false),
        ("LICENSE", false),
    ];
    for (i, &(name, folder)) in items.iter().enumerate() {
        let col = (i % 3) as i32;
        let row = (i / 3) as i32;
        let ix = x + 190 + col * 130;
        let iy = y + 20 + row * 110;
        if folder {
            g.round_rect(ix, iy, 34, 20, 5, 0x7cc4ff, 0x4a8be8, 255);
            g.round_rect(ix, iy + 10, 72, 52, 8, 0x5cb3ff, 0x2a62d6, 255);
        } else {
            g.round_rect(ix + 11, iy + 2, 50, 62, 6, 0xF2F2FA, 0xC9C9DC, 255);
            for k in 0..4 {
                g.rect(ix + 19, iy + 16 + k * 10, 34, 3, 0x8888a8, 255);
            }
        }
        let tw = text_width(name, 1);
        g.text(ix + 36 - tw / 2, iy + 72, name, 0xDDDDEE);
    }
}

fn settings(g: &mut Gfx, x: i32, y: i32, w: i32) {
    let rows = [("Dark mode", true), ("Animations", true), ("Sound", false), ("Network", true)];
    for (i, &(name, on)) in rows.iter().enumerate() {
        let ry = y + 20 + i as i32 * 52;
        g.text(x + 28, ry + 10, name, 0xE6E6F5);
        g.rect(x + 28, ry + 40, w - 56, 1, 0xFFFFFF, 18);
        let tx = x + w - 28 - 46;
        if on {
            g.round_rect(tx, ry + 4, 46, 26, 13, 0x8a6bff, 0x6a4bd8, 255);
        } else {
            g.round_rect(tx, ry + 4, 46, 26, 13, 0x55556a, 0x44445a, 255);
        }
        let kx = if on { tx + 33 } else { tx + 13 };
        g.circle(kx, ry + 17, 10, 0xFFFFFF, 255);
    }
}