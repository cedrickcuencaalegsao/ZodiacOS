//! Connects the userspace desktop library to the kernel's framebuffer, input and timer.
//! Add `pub mod desktop_host;` to kernel/src/console/mod.rs.
use crate::drivers::framebuffer::{self, Framebuffer};
use crate::drivers::mouse;
use crate::keyboard::{self, KEY_DOWN, KEY_LEFT, KEY_RIGHT, KEY_UP};
use alloc::vec;
use zodiac_desktop::{Event, Host, Key};

struct KernelHost {
    fb: Framebuffer,
    elapsed_ms: u64, // advanced by sleep_ms; replace with a real uptime clock later
    last_buttons: u8,
}

impl Host for KernelHost {
    fn size(&self) -> (usize, usize) {
        (self.fb.width, self.fb.height)
    }

    fn present(&mut self, px: &[u32]) {
        let (w, h) = (self.fb.width, self.fb.height);
        for y in 0..h {
            unsafe {
                let dst = (self.fb.base + y * self.fb.pitch) as *mut u32;
                core::ptr::copy_nonoverlapping(px.as_ptr().add(y * w), dst, w);
            }
        }
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
                    return Some(Event::Button { button: bit, down: now & (1 << bit) != 0 });
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

    // Drop keys left over from the login screen (e.g. the Enter that submitted the
    // password), otherwise the desktop would treat it as a click immediately.
    while keyboard::read_char().is_some() {}

    let has_mouse = mouse::init();
    crate::log_info!("mouse: {}", if has_mouse { "ready" } else { "not found (keyboard only)" });

    // ADAPT (optional): stop fbcon/kprintln from drawing over the desktop while it runs
    // crate::console::fbcon::suspend();

    let mut back = vec![0u32; fb.width * fb.height];
    let mut host = KernelHost { fb, elapsed_ms: 0, last_buttons: 0 };
    zodiac_desktop::run(&mut host, &mut back);

    mouse::shutdown(); // otherwise a stray mouse byte can block the keyboard at the login screen
    // crate::console::fbcon::resume();
    Ok(())
}