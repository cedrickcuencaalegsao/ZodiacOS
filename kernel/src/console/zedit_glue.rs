use crate::filesystem::file::{self, O_CREAT, O_RDONLY, O_TRUNC, O_WRONLY};
use crate::filesystem::FileType;
use crate::{keyboard, vga};
use alloc::{string::String, vec, vec::Vec};

// ---- Storage: files and directories through the kernel filesystem ----------

struct FsStorage;

impl zedit::Storage for FsStorage {
    fn read(&mut self, path: &str) -> Option<String> {
        let fd = file::open(path, O_RDONLY, 0).ok()?;
        let mut data: Vec<u8> = Vec::new();
        let mut buf = [0u8; 512];
        let ok = loop {
            match file::read(fd, &mut buf) {
                Ok(0) => break true,
                Ok(n) => data.extend_from_slice(&buf[..n]),
                Err(_) => break false,
            }
        };
        let _ = file::close(fd);
        if !ok {
            return None;
        }
        Some(String::from_utf8_lossy(&data).into_owned())
    }

    fn write(&mut self, path: &str, text: &str) -> bool {
        // O_TRUNC so :w replaces the file instead of appending
        let Ok(fd) = file::open(path, O_WRONLY | O_CREAT | O_TRUNC, 0o644) else {
            return false;
        };
        let bytes = text.as_bytes();
        let mut done = 0;
        let mut ok = true;
        while done < bytes.len() {
            match file::write(fd, &bytes[done..]) {
                Ok(0) | Err(_) => {
                    ok = false;
                    break;
                }
                Ok(n) => done += n,
            }
        }
        let _ = file::close(fd);
        ok
    }

    fn list_dir(&mut self, path: &str) -> Vec<(String, bool)> {
        let mut out = Vec::new();
        if let Ok(entries) = file::readdir(path) {
            for e in entries {
                out.push((e.name, e.kind == FileType::Directory));
            }
        }
        out
    }

    fn cwd(&mut self) -> String {
        file::getcwd()
    }
}

// ---- Screen: back buffer painted through the vga functions the shell uses --

struct VgaScreen {
    w: usize,
    h: usize,
    cells: Vec<(char, u8)>,
}

impl VgaScreen {
    fn new() -> Self {
        // one column less than the width, so the last cell never wraps or scrolls
        let w = vga::width().saturating_sub(1).max(20);
        let h = 24; // WIRE: use vga::height() if you have it
        VgaScreen {
            w,
            h,
            cells: vec![(' ', 0x07); w * h],
        }
    }
}

impl zedit::Screen for VgaScreen {
    fn size(&self) -> (usize, usize) {
        (self.w, self.h)
    }

    fn clear(&mut self) {
        for c in self.cells.iter_mut() {
            *c = (' ', 0x07);
        }
    }

    fn put(&mut self, x: usize, y: usize, c: char, color: u8) {
        if x < self.w && y < self.h {
            self.cells[y * self.w + x] = (c, color);
        }
    }

    fn flush(&mut self) {
        vga::clear();
        let mut last: i16 = -1;
        for y in 0..self.h {
            for x in 0..self.w {
                let (c, color) = self.cells[y * self.w + x];
                if color as i16 != last {
                    vga::set_color(color);
                    last = color as i16;
                }
                vga::put_byte(if c.is_ascii() && c >= ' ' { c as u8 } else { b'?' });
            }
            if y + 1 < self.h {
                vga::put_byte(b'\n');
            }
        }
        vga::set_color(0x0F);
    }
}

// ---- Keyboard ---------------------------------------------------------------

fn read_key() -> zedit::Key {
    use zedit::Key;
    loop {
        match keyboard::wait_char() {
            b'\n' | b'\r' => return Key::Enter,
            8 | 127 => return Key::Backspace,
            27 | 3 => return Key::Esc, // Esc, or Ctrl-C as a fallback
            c @ 0x20..=0x7E => return Key::Char(c as char),
            _ => {}
        }
    }
}

// ---- Entry point called from terminal.rs ------------------------------------

pub fn run(arg: Option<&str>) {
    let mut ed = zedit::Editor::new();
    let mut screen = VgaScreen::new();
    let mut store = FsStorage;

    match arg {
        Some(".") => ed.open_dir(".", &mut store),
        Some(path) => {
            ed.open(path, &mut store);
        }
        None => {}
    }

    loop {
        ed.render(&mut screen);
        if !ed.handle_key(read_key(), &mut store) {
            break;
        }
    }
}