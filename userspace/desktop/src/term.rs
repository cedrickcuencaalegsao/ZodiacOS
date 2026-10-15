//! The Terminal window's state: a text grid, the shell, and a filesystem.
use crate::gfx::{text_width, Gfx};
use crate::memfs::MemFs;
use alloc::{string::String, vec, vec::Vec};
use zodiac_abi::{Action, Console, Key};
use zodiac_shell::Shell;

const LINE_H: i32 = 18;
const MAX_LINES: usize = 500;

/// Scrollback buffer. Output always appends at the end of the last line.
pub struct Screen {
    lines: Vec<Vec<char>>,
    cols: usize,
}

impl Screen {
    fn new() -> Self {
        Screen { lines: vec![Vec::new()], cols: 60 }
    }

    fn backspace(&mut self) {
        // If a wrapped line is empty, step back onto the previous one.
        if self.lines.len() > 1 && self.lines.last().map_or(false, |l| l.is_empty()) {
            self.lines.pop();
        }
        if let Some(l) = self.lines.last_mut() {
            l.pop();
        }
    }
}

impl Console for Screen {
    fn write_str(&mut self, s: &str) {
        for c in s.chars() {
            match c {
                '\n' => {
                    self.lines.push(Vec::new());
                    if self.lines.len() > MAX_LINES {
                        self.lines.remove(0);
                    }
                }
                '\x08' => self.backspace(),
                c => {
                    let full = self.lines.last().map_or(true, |l| l.len() >= self.cols);
                    if full {
                        self.lines.push(Vec::new());
                    }
                    if let Some(l) = self.lines.last_mut() {
                        l.push(c);
                    }
                }
            }
        }
    }

    fn clear(&mut self) {
        self.lines.clear();
        self.lines.push(Vec::new());
    }
}

pub struct TermState {
    screen: Screen,
    shell: Shell,
    fs: MemFs,
}

impl TermState {
    pub fn new() -> Self {
        let mut screen = Screen::new();
        let mut shell = Shell::new();
        shell.start(&mut screen);
        TermState { screen, shell, fs: MemFs::new() }
    }

    /// Feed a key press. `secs` is seconds since boot (used for file timestamps).
    pub fn on_key(&mut self, key: Key, secs: u64) -> Action {
        self.fs.now = secs;
        self.shell.on_key(key, &mut self.screen, &mut self.fs)
    }

    pub fn draw(&mut self, g: &mut Gfx, x: i32, y: i32, w: i32, h: i32, secs: u64) {
        g.round_rect(x + 10, y + 4, w - 20, h - 14, 8, 0x0b0b12, 0x0b0b12, 255);

        let cw = text_width("M", 1).max(1);
        self.screen.cols = ((w - 48) / cw).max(8) as usize;
        let rows = ((h - 14 - 24) / LINE_H).max(1) as usize;

        let lines = &self.screen.lines;
        let start = lines.len().saturating_sub(rows);
        let last_idx = lines.len() - 1;

        for (i, chars) in lines[start..].iter().enumerate() {
            let s: String = chars.iter().collect();
            let ly = y + 16 + i as i32 * LINE_H;
            let is_prompt = s.contains("@zodiacos");
            match s.find("> ") {
                Some(idx) if is_prompt => {
                    let (prompt, rest) = s.split_at(idx + 2);
                    g.text(x + 24, ly, prompt, 0x6CFF9C);
                    g.text(x + 24 + text_width(prompt, 1), ly, rest, 0xE6E6F5);
                }
                _ => g.text(x + 24, ly, &s, 0xC9C9E0),
            }

            // Blinking cursor at the end of the last line.
            if start + i == last_idx && secs % 2 == 0 {
                g.rect(x + 24 + text_width(&s, 1), ly, 8, 16, 0x6CFF9C, 255);
            }
        }
    }
}