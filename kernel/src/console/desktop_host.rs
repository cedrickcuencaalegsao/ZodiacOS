#![allow(dead_code)]
//! Connects the userspace desktop library to the kernel's framebuffer, input and timer.
//! Add `pub mod desktop_host;` to kernel/src/console/mod.rs.
use crate::drivers::framebuffer::{self, Framebuffer};
use crate::drivers::mouse;
use crate::keyboard::{self, KEY_DOWN, KEY_LEFT, KEY_RIGHT, KEY_UP};
use alloc::vec;
use alloc::vec::Vec;
use zodiac_desktop::{Event, Host, Key};

/// The desktop asks for up to this many screens of scratch memory (width * height pixels each)
/// and uses as many as the kernel heap can spare:
/// 3 = frame + finished scene + cached wallpaper: pointer movement repaints only the cursor.
/// 2 = wallpaper cached only. 1 = minimal (everything is redrawn on every event).
const MAX_BUFFERS: usize = 3;

/// A pixel value the desktop never produces (it draws 0x00RRGGBB), meaning "unknown on screen".
const UNKNOWN: u32 = 0xFFFF_FFFF;

// ---- presenter begin -------------------------------------------------------------
// Gets finished frames onto the screen without the monitor ever seeing a half-written one.
//
// With two pages in video memory, a frame is written to the page that is NOT being shown and
// the card is then told to show it (one register write). That page is one frame old, so besides
// the pixels that changed in this frame it also receives the ones that changed in the previous
// frame. With a single page it falls back to writing only the changed pixels in place.

/// (start, end) columns of one row, end exclusive; start >= end means "nothing".
type Span = (usize, usize);
const NONE: Span = (0, 0);

fn join(a: Span, b: Span) -> Span {
    if a.0 >= a.1 {
        b
    } else if b.0 >= b.1 {
        a
    } else {
        (a.0.min(b.0), a.1.max(b.1))
    }
}

struct Presenter {
    base: usize,  // address of page 0
    pitch: usize, // bytes per row
    w: usize,
    h: usize,
    pages: usize,   // 2 = page flipping, 1 = single buffer
    visible: usize, // the page the monitor shows
    /// The frame the visible page holds. Empty if the heap had no room: whole rows are written.
    shown: Vec<u32>,
    /// Per row: what the previous frame changed (the hidden page is still missing it).
    prev: Vec<Span>,
    next: Vec<Span>,
}

impl Presenter {
    /// Puts the part of `px` inside [x0, x1) x [y0, y1) on screen. Pixels of `px` outside that
    /// rectangle must be the same as in the previous frame.
    /// Returns Some(page) when the caller has to make that page visible now.
    fn present(&mut self, px: &[u32], x0: usize, y0: usize, x1: usize, y1: usize) -> Option<usize> {
        let (w, h) = (self.w, self.h);
        if px.len() < w * h {
            return None;
        }
        let (x1, y1) = (x1.min(w), y1.min(h));
        let flip = self.pages > 1;
        let diff = !self.shown.is_empty();
        let target = if flip { 1 - self.visible } else { self.visible };
        let page = self.base + target * h * self.pitch;
        let mut changed = false;

        for y in 0..h {
            let row = y * w;

            // what is new on this row
            let mut now = NONE;
            if y >= y0 && y < y1 && x0 < x1 {
                if diff {
                    let new = &px[row + x0..row + x1];
                    let old = &self.shown[row + x0..row + x1];
                    if let Some(f) = new.iter().zip(old).position(|(a, b)| a != b) {
                        let l = new.iter().zip(old).rposition(|(a, b)| a != b).unwrap_or(f);
                        now = (x0 + f, x0 + l + 1);
                    }
                } else {
                    now = (x0, x1);
                }
            }
            changed |= now.0 < now.1;

            // what the target page needs: this change, plus last frame's if it is a hidden page
            let todo = if flip { join(now, self.prev[y]) } else { now };
            if todo.0 < todo.1 {
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        px.as_ptr().add(row + todo.0),
                        (page + y * self.pitch + todo.0 * 4) as *mut u32,
                        todo.1 - todo.0,
                    );
                }
            }
            if diff && now.0 < now.1 {
                self.shown[row + now.0..row + now.1].copy_from_slice(&px[row + now.0..row + now.1]);
            }
            if flip {
                self.next[y] = now;
            }
        }

        if !flip {
            return None;
        }
        if changed {
            core::mem::swap(&mut self.prev, &mut self.next);
            self.visible = target;
            Some(target)
        } else {
            // nothing new: the hidden page just caught up with the visible one
            self.prev.iter_mut().for_each(|s| *s = NONE);
            None
        }
    }
}
// ---- presenter end ---------------------------------------------------------------

struct KernelHost {
    fb: Framebuffer,
    elapsed_ms: u64, // advanced by sleep_ms; replace with a real uptime clock later
    last_buttons: u8,
    out: Presenter,
}

/// Allocates `len` pixels without panicking when the heap is too small.
fn try_buffer(len: usize, fill: u32) -> Option<Vec<u32>> {
    let mut v: Vec<u32> = Vec::new();
    v.try_reserve_exact(len).ok()?;
    v.resize(len, fill); // capacity is already exact, so this cannot allocate again
    Some(v)
}

impl KernelHost {
    fn show(&mut self, px: &[u32], x0: usize, y0: usize, x1: usize, y1: usize) {
        if let Some(page) = self.out.present(px, x0, y0, x1, y1) {
            framebuffer::show_page(page);
        }
    }
}

impl Host for KernelHost {
    fn size(&self) -> (usize, usize) {
        (self.fb.width, self.fb.height)
    }

    fn present(&mut self, px: &[u32]) {
        let (w, h) = (self.fb.width, self.fb.height);
        self.show(px, 0, 0, w, h);
    }

    fn present_rect(&mut self, px: &[u32], x: usize, y: usize, w: usize, h: usize) {
        self.show(px, x, y, x + w, y + h);
    }

    fn poll_event(&mut self) -> Option<Event> {
        // 1. mouse: motion first, then button changes (so a click lands where the pointer is)
        mouse::poll();
        let (dx, dy) = mouse::take_motion();
        if dx != 0 || dy != 0 {
            return Some(Event::MouseMove { dx, dy });
        }
        let now = mouse::buttons();
        let changed = now ^ self.last_buttons;
        if changed != 0 {
            for bit in 0..3u8 {
                if changed & (1 << bit) != 0 {
                    self.last_buttons ^= 1 << bit;
                    return Some(Event::Button {
                        button: bit,
                        down: now & (1 << bit) != 0,
                    });
                }
            }
        }

        // 2. keyboard
        let c = keyboard::read_char()?;
        let key = match c {
            KEY_UP => Key::Up,
            KEY_DOWN => Key::Down,
            KEY_LEFT => Key::Left,
            KEY_RIGHT => Key::Right,
            b'\n' => Key::Enter,
            27 => Key::Escape,
            c if c < 0x80 => Key::Char(c),
            _ => return None, // Home/End/PageUp/PageDown/Delete are unused for now
        };
        Some(Event::Key { key, down: true })
    }

    fn uptime_ms(&mut self) -> u64 {
        self.elapsed_ms
    }

    fn sleep_ms(&mut self, ms: u64) {
        crate::scheduler::sleep_ms(ms as _);
        self.elapsed_ms += ms;
    }

    fn font(&self) -> &'static [[u8; 16]; 256] {
        framebuffer::font_table()
    }
}

/// Shows the desktop on the framebuffer. Returns when the user logs out.
pub fn run() -> Result<(), &'static str> {
    let fb = framebuffer::get().ok_or("no framebuffer: still in VGA text mode")?;

    // Memory first, so running out of heap is a clean error instead of a kernel panic.
    // Priority: the "what is on screen" copy, then as many scratch screens for the desktop
    // as will fit (1 is required, 3 is best).
    let pixels = (fb.width * fb.height).max(1);
    let shown = try_buffer(pixels, UNKNOWN).unwrap_or_default();
    let mut back: Vec<u32> = Vec::new();
    for n in (1..=MAX_BUFFERS).rev() {
        if let Some(v) = try_buffer(pixels * n, 0) {
            back = v;
            break;
        }
    } 
    if back.is_empty(){
        return Err("not enough kernel heap fo the desktop");
    }
    let pages = framebuffer::pages();
    crate::log_info!(
        "desktop: {} scratch screen(s), {} video page(s), {}",
        back.len() / pixels,
        pages,
        if shown.is_empty() {
            "full-row writes"
        } else {
            "changed-pixels writes"
        }
    );

    // Drop keys left over from the login screen (e.g. the Enter that submitted the
    // password), otherwise the desktop would treat it as a click immediately.
    while keyboard::read_char().is_some() {}

    let has_mouse = mouse::init();
    crate::log_info!(
        "mouse: {}",
        if has_mouse {
            "ready"
        } else {
            "not found (keyboard only)"
        }
    );

    // ADAPT (optional): stop fbcon/kprintln from drawing over the desktop while it runs
    // crate::console::fbcon::suspend();

    let out = Presenter {
        base: fb.base,
        pitch: fb.pitch,
        w: fb.width,
        h: fb.height,
        pages,
        visible: 0,
        shown,
        prev: vec![NONE; fb.height],
        next: vec![NONE; fb.height],
    };
    let mut host = KernelHost {
        fb,
        elapsed_ms: 0,
        last_buttons: 0,
        out,
    };
    zodiac_desktop::run(&mut host, back.as_mut_slice());

    framebuffer::show_page(0); // the console draws on page 0
    mouse::shutdown(); // otherwise a stray mouse byte can block the keyboard at the login screen
    // crate::console::fbcon::resume();
    Ok(())
}
