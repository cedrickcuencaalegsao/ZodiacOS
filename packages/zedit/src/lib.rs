#![cfg_attr(not(test), no_std)]
extern crate alloc;

use alloc::{format, string::String, vec::Vec};

mod config;
mod file;

pub use file::FileKind;
use config::{parse_line, LineKind};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Key {
    Char(char),
    Enter,
    Backspace,
    Esc,
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    Normal,
    Insert,
    Command,
    Browse,
}

/// Implemented by the kernel. Zedit never touches hardware directly.
pub trait Screen {
    fn size(&self) -> (usize, usize);
    fn clear(&mut self);
    fn put(&mut self, x: usize, y: usize, c: char, color: u8);
    fn flush(&mut self);
}

pub trait Storage {
    fn read(&mut self, path: &str) -> Option<String>;
    fn write(&mut self, path: &str, data: &str) -> bool;
    fn list_dir(&mut self, path: &str) -> Vec<(String, bool)>; // (name, is_dir)
    fn cwd(&mut self) -> String;
}

const C_TEXT: u8 = 0x07;
const C_BAR: u8 = 0x70;

pub struct Editor {
    lines: Vec<String>,
    cx: usize,
    cy: usize,
    top: usize,
    mode: Mode,
    return_mode: Mode,
    path: Option<String>,
    kind: FileKind,
    dirty: bool,
    pending: Option<char>,
    cmd: String,
    msg: String,
    browse_dir: String,
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

fn join(dir: &str, name: &str) -> String {
    if dir == "." {
        String::from(name)
    } else if dir.ends_with('/') {
        format!("{}{}", dir, name)
    } else {
        format!("{}/{}", dir, name)
    }
}

/// Keep the buffer plain ASCII so byte index == column.
fn sanitize(text: &str) -> Vec<String> {
    let mut out: Vec<String> = text
        .lines()
        .map(|l| {
            let mut s = String::new();
            for c in l.chars() {
                match c {
                    '\t' => s.push_str("    "),
                    c if c.is_ascii() && !c.is_ascii_control() => s.push(c),
                    _ => s.push('?'),
                }
            }
            s
        })
        .collect();
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

fn put_str<S: Screen>(s: &mut S, x: usize, y: usize, t: &str, color: u8) {
    for (i, c) in t.chars().enumerate() {
        s.put(x + i, y, c, color);
    }
}

impl Editor {
    pub fn new() -> Self {
        Editor {
            lines: alloc::vec![String::new()],
            cx: 0,
            cy: 0,
            top: 0,
            mode: Mode::Normal,
            return_mode: Mode::Normal,
            path: None,
            kind: FileKind::Text,
            dirty: false,
            pending: None,
            cmd: String::new(),
            msg: String::new(),
            browse_dir: String::new(),
        }
    }

    // ---- opening / saving ----

    /// Opens a file. A missing file starts an empty buffer that `:w` will create.
    pub fn open(&mut self, path: &str, store: &mut impl Storage) -> bool {
        self.mode = Mode::Normal;
        self.path = Some(String::from(path));
        self.kind = FileKind::from_path(path);
        self.cx = 0;
        self.cy = 0;
        self.top = 0;
        self.dirty = false;
        match store.read(path) {
            Some(text) => {
                self.lines = sanitize(&text);
                self.msg = format!("\"{}\" {}L", path, self.lines.len());
                true
            }
            None => {
                self.lines = alloc::vec![String::new()];
                self.msg = String::from("[new file]");
                false
            }
        }
    }

    pub fn open_dir(&mut self, path: &str, store: &mut impl Storage) {
        let mut entries = store.list_dir(path);
        entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let mut lines = alloc::vec![String::from("../")];
        for (name, is_dir) in entries {
            if name == "." || name == ".." {
                continue;
            }
            lines.push(if is_dir { format!("{}/", name) } else { name });
        }
        self.lines = lines;
        self.browse_dir = String::from(path);
        self.path = None;
        self.kind = FileKind::Text;
        self.mode = Mode::Browse;
        self.cx = 0;
        self.cy = 0;
        self.top = 0;
        self.dirty = false;
        self.msg = String::from("j/k move, Enter open, :q quit");
    }

    fn save(&mut self, arg: Option<&str>, store: &mut impl Storage) -> bool {
        let path = match arg {
            Some(p) => String::from(p),
            None => match &self.path {
                Some(p) => p.clone(),
                None => {
                    self.msg = String::from("no file name (use :w <name>)");
                    return false;
                }
            },
        };
        let mut text = self.lines.join("\n");
        text.push('\n');
        if store.write(&path, &text) {
            self.msg = format!("\"{}\" {}L written", path, self.lines.len());
            self.kind = FileKind::from_path(&path);
            self.path = Some(path);
            self.dirty = false;
            true
        } else {
            self.msg = format!("cannot write \"{}\"", path);
            false
        }
    }

    // ---- input ----

    /// Returns false when the editor should exit.
    pub fn handle_key(&mut self, key: Key, store: &mut impl Storage) -> bool {
        let keep = match self.mode {
            Mode::Normal => {
                self.normal(key);
                true
            }
            Mode::Insert => {
                self.insert(key);
                true
            }
            Mode::Command => self.command(key, store),
            Mode::Browse => self.browse(key, store),
        };
        self.clamp();
        keep
    }

    fn clamp(&mut self) {
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        if self.cy >= self.lines.len() {
            self.cy = self.lines.len() - 1;
        }
        let len = self.lines[self.cy].len();
        let max = if self.mode == Mode::Insert { len } else { len.saturating_sub(1) };
        if self.cx > max {
            self.cx = max;
        }
    }

    fn start_command(&mut self) {
        self.return_mode = self.mode;
        self.mode = Mode::Command;
        self.cmd.clear();
        self.msg.clear();
    }

    fn normal(&mut self, key: Key) {
        let pending = self.pending.take();
        match key {
            Key::Char('h') | Key::Left => self.cx = self.cx.saturating_sub(1),
            Key::Char('l') | Key::Right => self.cx += 1,
            Key::Char('j') | Key::Down => self.cy += 1,
            Key::Char('k') | Key::Up => self.cy = self.cy.saturating_sub(1),
            Key::Char('0') => self.cx = 0,
            Key::Char('$') => self.cx = self.lines[self.cy].len(),
            Key::Char('G') => self.cy = self.lines.len() - 1,
            Key::Char('g') => {
                if pending == Some('g') {
                    self.cy = 0;
                } else {
                    self.pending = Some('g');
                }
            }
            Key::Char('i') => self.mode = Mode::Insert,
            Key::Char('a') => {
                self.mode = Mode::Insert;
                if !self.lines[self.cy].is_empty() {
                    self.cx += 1;
                }
            }
            Key::Char('A') => {
                self.mode = Mode::Insert;
                self.cx = self.lines[self.cy].len();
            }
            Key::Char('o') => {
                self.lines.insert(self.cy + 1, String::new());
                self.cy += 1;
                self.cx = 0;
                self.mode = Mode::Insert;
                self.dirty = true;
            }
            Key::Char('O') => {
                self.lines.insert(self.cy, String::new());
                self.cx = 0;
                self.mode = Mode::Insert;
                self.dirty = true;
            }
            Key::Char('x') => {
                if self.cx < self.lines[self.cy].len() {
                    self.lines[self.cy].remove(self.cx);
                    self.dirty = true;
                }
            }
            Key::Char('d') => {
                if pending == Some('d') {
                    self.lines.remove(self.cy);
                    if self.lines.is_empty() {
                        self.lines.push(String::new());
                    }
                    self.dirty = true;
                } else {
                    self.pending = Some('d');
                }
            }
            Key::Char(':') => self.start_command(),
            _ => {}
        }
    }

    fn insert(&mut self, key: Key) {
        match key {
            Key::Esc => {
                self.mode = Mode::Normal;
                self.cx = self.cx.saturating_sub(1);
            }
            Key::Char(c) if c.is_ascii() && !c.is_ascii_control() => {
                self.lines[self.cy].insert(self.cx, c);
                self.cx += 1;
                self.dirty = true;
            }
            Key::Enter => {
                let rest = self.lines[self.cy].split_off(self.cx);
                self.lines.insert(self.cy + 1, rest);
                self.cy += 1;
                self.cx = 0;
                self.dirty = true;
            }
            Key::Backspace => {
                if self.cx > 0 {
                    self.lines[self.cy].remove(self.cx - 1);
                    self.cx -= 1;
                    self.dirty = true;
                } else if self.cy > 0 {
                    let cur = self.lines.remove(self.cy);
                    self.cy -= 1;
                    self.cx = self.lines[self.cy].len();
                    self.lines[self.cy].push_str(&cur);
                    self.dirty = true;
                }
            }
            Key::Left => self.cx = self.cx.saturating_sub(1),
            Key::Right => self.cx += 1,
            Key::Up => self.cy = self.cy.saturating_sub(1),
            Key::Down => self.cy += 1,
            _ => {}
        }
    }

    fn command(&mut self, key: Key, store: &mut impl Storage) -> bool {
        match key {
            Key::Esc => self.mode = self.return_mode,
            Key::Char(c) if c.is_ascii() && !c.is_ascii_control() => self.cmd.push(c),
            Key::Backspace => {
                if self.cmd.pop().is_none() {
                    self.mode = self.return_mode;
                }
            }
            Key::Enter => {
                self.mode = self.return_mode;
                return self.run_command(store);
            }
            _ => {}
        }
        true
    }

    fn run_command(&mut self, store: &mut impl Storage) -> bool {
        let cmd = core::mem::take(&mut self.cmd);
        let mut it = cmd.split_whitespace();
        let name = it.next().unwrap_or("");
        let arg = it.next();
        match name {
            "" => true,
            "w" => {
                self.save(arg, store);
                true
            }
            "wq" | "x" => !self.save(arg, store),
            "q" => {
                if self.dirty {
                    self.msg = String::from("unsaved changes (:q! to discard, :wq to save)");
                    true
                } else {
                    false
                }
            }
            "q!" => false,
            other => {
                self.msg = format!("unknown command: {}", other);
                true
            }
        }
    }

    fn browse(&mut self, key: Key, store: &mut impl Storage) -> bool {
        self.cx = 0;
        match key {
            Key::Char('j') | Key::Down => self.cy += 1,
            Key::Char('k') | Key::Up => self.cy = self.cy.saturating_sub(1),
            Key::Char(':') => self.start_command(),
            Key::Char('q') => return false,
            Key::Enter => {
                self.clamp();
                let name = self.lines[self.cy].clone();
                let dir = self.browse_dir.clone();
                if name.ends_with('/') {
                    let target = join(&dir, name.trim_end_matches('/'));
                    self.open_dir(&target, store);
                } else {
                    let target = join(&dir, &name);
                    self.open(&target, store);
                }
            }
            _ => {}
        }
        true
    }

    // ---- drawing ----

    pub fn render<S: Screen>(&mut self, s: &mut S) {
        let (w, h) = s.size();
        if w < 10 || h < 4 {
            return;
        }
        let rows = h - 2;

        if self.cy < self.top {
            self.top = self.cy;
        }
        if self.cy >= self.top + rows {
            self.top = self.cy + 1 - rows;
        }
        let left = if self.cx >= w { self.cx + 1 - w } else { 0 };

        s.clear();

        for r in 0..rows {
            let li = self.top + r;
            if li < self.lines.len() {
                self.draw_line(s, r, li, left, w);
            } else {
                s.put(0, r, '~', 0x01);
            }
        }

        // status bar
        for x in 0..w {
            s.put(x, rows, ' ', C_BAR);
        }
        let mode = match self.mode {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Command => "COMMAND",
            Mode::Browse => "BROWSE",
        };
        let name = if self.mode == Mode::Browse {
            self.browse_dir.clone()
        } else {
            self.path.clone().unwrap_or_else(|| String::from("[No Name]"))
        };
        let mut st = format!(" {} | {}{}", mode, name, if self.dirty { " [+]" } else { "" });
        if self.kind == FileKind::Config {
            let errs = self.lines.iter().filter(|l| parse_line(l) == LineKind::Invalid).count();
            if errs > 0 {
                st.push_str(&format!(" | {} error(s)", errs));
            }
        }
        put_str(s, 0, rows, &st, C_BAR);
        let pos = format!("{}:{} ", self.cy + 1, self.cx + 1);
        if pos.len() + st.len() < w {
            put_str(s, w - pos.len(), rows, &pos, C_BAR);
        }

        // message / command line
        if self.mode == Mode::Command {
            put_str(s, 0, h - 1, &format!(":{}", self.cmd), C_TEXT);
        } else {
            put_str(s, 0, h - 1, &self.msg, C_TEXT);
        }

        // cursor
        let (cx, cy) = if self.mode == Mode::Command {
            (1 + self.cmd.len(), h - 1)
        } else {
            (self.cx - left, self.cy - self.top)
        };
        let ch = if self.mode == Mode::Command {
            ' '
        } else {
            self.lines[self.cy].chars().nth(self.cx).unwrap_or(' ')
        };
        s.put(cx, cy, ch, C_BAR);

        s.flush();
    }

    fn draw_line<S: Screen>(&self, s: &mut S, r: usize, li: usize, left: usize, w: usize) {
        let line = &self.lines[li];
        let base = if self.mode == Mode::Browse {
            if line.ends_with('/') { 0x0B } else { 0x0F }
        } else {
            C_TEXT
        };
        let parsed = if self.kind == FileKind::Config && self.mode != Mode::Browse {
            Some(parse_line(line))
        } else {
            None
        };
        let eq = line.find('=');
        for (i, ch) in line.chars().enumerate() {
            if i < left {
                continue;
            }
            let x = i - left;
            if x >= w {
                break;
            }
            let color = match &parsed {
                Some(LineKind::Comment) => 0x08,
                Some(LineKind::Section(_)) => 0x0E,
                Some(LineKind::Invalid) => 0x0C,
                Some(LineKind::Pair { .. }) => match eq {
                    Some(e) if i > e => 0x0F,
                    Some(e) if i == e => 0x07,
                    _ => 0x0B,
                },
                _ => base,
            };
            s.put(x, r, ch, color);
        }
    }
}