#![allow(dead_code)]
//! kernel/src/console/login.rs
//!
//! Adapter between the `login_manager` UI crate and the kernel's
//! `security::service` (account database + session manager).

use alloc::{string::String, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};
use login_manager::session::{AuthError, Session};
use login_manager::user_accounts::UserInfo;
use login_manager::{Color, Gfx, Key, Platform};
use spin::Mutex;

use crate::console::fbcon;
use crate::keyboard;
use crate::security::auth::AuthError as KAuthError;
use crate::security::service;

const PASSWD_PATH: &str = "/etc/passwd";
const PASSWD_MODE: u16 = 0o600;
/// Dev shortcut: pre-create root so first-boot setup is skipped.
/// Passwords must be 8+ characters now, so this is "rootroot", not "root".
const DEV_DEFAULT_ROOT: bool = false;

// Fallback VGA text mode (only used if the framebuffer console isn't active)
const VGA_COLS: usize = 80;
const VGA_ROWS: usize = 25;
const VGA_BUFFER: usize = 0xB8000;

/// Same colors as fbcon's PALETTE, indexed by `Color as u8`.
const PALETTE: [u32; 16] = [
    0x000000, 0x0000AA, 0x00AA00, 0x00AAAA, 0xAA0000, 0xAA00AA, 0xAA5500, 0xAAAAAA, 0x555555,
    0x5555FF, 0x55FF55, 0x55FFFF, 0xFF5555, 0xFF55FF, 0xFFFF55, 0xFFFFFF,
];

/// The recovery code from first-run setup. The login UI has no screen for it, so it
/// waits here until the caller shows it with `take_recovery_code()`.
static RECOVERY_CODE: Mutex<Option<String>> = Mutex::new(None);
static INIT_DONE: AtomicBool = AtomicBool::new(false);

/// Returns the recovery code once (first-run setup only), then None.
pub fn take_recovery_code() -> Option<String> {
    RECOVERY_CODE.lock().take()
}

fn map_err(e: KAuthError) -> AuthError {
    match e {
        KAuthError::AlreadySetUp => AuthError::AlreadySetUp,
        KAuthError::WeakPassword => AuthError::WeakPassword,
        // Fail closed: never create accounts with weak salts.
        KAuthError::NoEntropy => panic!("no hardware RNG; run QEMU with -cpu max"),
        _ => AuthError::InvalidCredentials,
    }
}

/// Timestamp stored in the session. ADAPT: swap for your tick counter.
fn now() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        unsafe { core::arch::x86_64::_rdtsc() }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        0
    }
}

struct KernelPlatform;

impl Platform for KernelPlatform {
    // ---- display: goes through the framebuffer console ----
    fn size(&self) -> (usize, usize) {
        if fbcon::active() {
            (fbcon::cols(), fbcon::rows())
        } else {
            (VGA_COLS, VGA_ROWS)
        }
    }

    fn put_char(&mut self, x: usize, y: usize, ch: char, fg: Color, bg: Color) {
        let byte = if ch.is_ascii() { ch as u8 } else { b'?' };
        let attr = ((bg as u8) << 4) | (fg as u8);

        if fbcon::active() {
            fbcon::put_char(y, x, byte, attr); // (row, col, ch, attr)
        } else if x < VGA_COLS && y < VGA_ROWS {
            unsafe {
                let cell = (VGA_BUFFER as *mut u16).add(y * VGA_COLS + x);
                cell.write_volatile(((attr as u16) << 8) | byte as u16);
            }
        }
    }

    fn clear(&mut self, bg: Color) {
        if fbcon::active() {
            // Fast pixel fill; every cell we draw afterwards updates the text grid.
            let (w, h) = fbcon::pixel_size();
            fbcon::fill_px(0, 0, w, h, PALETTE[bg as usize]);
        } else {
            for y in 0..VGA_ROWS {
                for x in 0..VGA_COLS {
                    self.put_char(x, y, ' ', Color::White, bg);
                }
            }
        }
    }

    // ---- input / time ----
    fn read_key(&mut self) -> Key {
        loop {
            if let Some(b) = keyboard::read_char() {
                return match b {
                    b'\n' | b'\r' => Key::Enter,
                    0x08 | 0x7f => Key::Backspace,
                    b'\t' => Key::Tab,
                    0x1b => Key::Esc,
                    0x20..=0x7e => Key::Char(b as char),
                    _ => continue,
                };
            }
            core::hint::spin_loop();
        }
    }

    fn sleep_ms(&mut self, ms: u64) {
        crate::arch::sleep_ms(ms as _);
    }

    // ---- account services: adapters over security::service ----
    fn has_accounts(&self) -> bool {
        !service::needs_setup()
    }

    fn list_users(&self) -> Vec<UserInfo> {
        service::list_users()
            .into_iter()
            .map(|u| UserInfo { name: u.name, uid: u.uid })
            .collect()
    }

    fn authenticate(&mut self, name: &str, password: &str) -> Result<Session, AuthError> {
        service::login(name, password, now()).map_err(map_err)?;
        let s = service::whoami().ok_or(AuthError::InvalidCredentials)?;
        Ok(Session { uid: s.uid, gid: s.gid, name: s.name, home: s.home })
    }

    fn create_root(&mut self, password: &str) -> Result<(), AuthError> {
        let code = service::first_run_setup(password).map_err(map_err)?;
        *RECOVERY_CODE.lock() = Some(code);
        Ok(())
    }

    fn gfx(&mut self) -> Option<&mut dyn Gfx> {
        if fbcon::active() { Some(self) } else { None }
    }
}

// ---- pixel drawing for the modern login screen: forwards to fbcon ---------------
impl Gfx for KernelPlatform {
    fn size_px(&self) -> (usize, usize) {
        fbcon::pixel_size()
    }

    fn fill(&mut self, x: usize, y: usize, w: usize, h: usize, rgb: u32) {
        fbcon::fill_px(x, y, w, h, rgb)
    }

    fn gradient(&mut self, x: usize, y: usize, w: usize, h: usize, top: u32, bottom: u32) {
        fbcon::gradient(x, y, w, h, top, bottom)
    }

    fn round_rect(
        &mut self,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
        r: usize,
        rgb: u32,
        alpha: u8,
    ) {
        fbcon::round_rect(x, y, w, h, r, rgb, alpha)
    }

    fn circle(&mut self, cx: usize, cy: usize, r: usize, rgb: u32, alpha: u8) {
        fbcon::circle(cx, cy, r, rgb, alpha)
    }

    fn text(&mut self, x: usize, y: usize, s: &str, scale: usize, rgb: u32) {
        fbcon::text(x, y, s, scale, rgb)
    }

    fn text_width(&self, s: &str, scale: usize) -> usize {
        fbcon::text_width(s, scale)
    }

    fn blit_mask(&mut self, x: usize, y: usize, w: usize, h: usize, mask: &[u8], rgb: u32) {
        fbcon::blit_mask(x, y, w, h, mask, rgb)
    }
}

/// TODO: wire to your zdfs read function; return the file bytes or None.
fn read_passwd() -> Option<Vec<u8>> {
    // crate::fs::read_file(PASSWD_PATH).ok()
    None
}

pub fn run() -> Session {
    // Hide the text cursor while our screens are drawn.
    crate::vga::disable_cursor();
    if fbcon::active() {
        fbcon::set_cursor(false);
    }

    // Load accounts once. run() is called again after every logout, and a second
    // init() would wipe the in-memory accounts.
    if !INIT_DONE.swap(true, Ordering::SeqCst) {
        service::init(read_passwd());
    }

    if DEV_DEFAULT_ROOT && service::needs_setup() {
        if let Ok(code) = service::first_run_setup("rootroot") {
            *RECOVERY_CODE.lock() = Some(code);
        }
    }

    let session = login_manager::run(&mut KernelPlatform);

    if let Some(file) = service::take_pending_save() {
        let _ = file;
        // let _ = crate::fs::write(PASSWD_PATH, PASSWD_MODE, file.as_bytes());
    }
    session
}