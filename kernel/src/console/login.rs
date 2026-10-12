use alloc::string::String;
use alloc::vec::Vec;
use login_manager::{
    authentication::make_account, greeter, session::Session, user_accounts::AccountDb, Platform,
};

use crate::{keyboard, vga};

/// Set to true to skip first-boot setup and use root/root while developing.
const DEV_DEFAULT_ROOT: bool = false;

const PASSWD_PATH: &str = "/etc/passwd";
/// Mode bits for the password file (owner read/write only).
const PASSWD_MODE: u16 = 0o600;

struct KernelPlatform;

impl Platform for KernelPlatform {
    fn write_str(&mut self, s: &str) {
        crate::kprint!("{}", s);
    }

    fn clear(&mut self) {
        vga::clear();
    }

    fn read_line(&mut self, echo: bool) -> String {
        let mut line = String::new();
        loop {
            // keyboard::read_char() is non-blocking (Option<u8>), so poll.
            let byte = loop {
                if let Some(b) = keyboard::read_char() {
                    break b;
                }
                core::hint::spin_loop();
            };
            let c = byte as char;

            match c {
                '\n' | '\r' => {
                    crate::kprint!("\n");
                    return line;
                }
                '\u{8}' | '\u{7f}' => {
                    if line.pop().is_some() && echo {
                        crate::kprint!("\u{8} \u{8}");
                    }
                }
                c if !c.is_control() => {
                    line.push(c);
                    if echo {
                        crate::kprint!("{}", c);
                    }
                }
                _ => {}
            }
        }
    }

    fn random_bytes(&mut self, buf: &mut [u8]) {
        // Weak but fine for salts. Swap for RDRAND when available.
        for b in buf.iter_mut() {
            let t = unsafe { core::arch::x86_64::_rdtsc() };
            *b = (t ^ (t >> 17) ^ (t >> 33)) as u8;
            for _ in 0..50 {
                core::hint::spin_loop();
            }
        }
    }

    fn sleep_ms(&mut self, ms: u64) {
        // `as _` adapts to whatever integer type arch::sleep_ms takes.
        crate::arch::sleep_ms(ms as _);
    }
}

/// TODO: wire this to your zdfs read function.
/// Return the raw bytes of the file, or None if it doesn't exist.
fn read_passwd() -> Option<Vec<u8>> {
    // Example once you know the API:
    // crate::fs::read_file(PASSWD_PATH).ok()
    None
}

fn load_accounts() -> AccountDb {
    if DEV_DEFAULT_ROOT {
        let mut db = AccountDb::default();
        db.add(make_account("root", "root", 0, 0, [1; 16]));
        db.dirty = false;
        return db;
    }

    match read_passwd().and_then(|bytes| String::from_utf8(bytes).ok()) {
        Some(text) => AccountDb::parse(&text),
        None => AccountDb::default(),
    }
}

fn save_accounts(db: &AccountDb) {
    // zdfs::write takes (path, mode: u16, data).
    let _ = crate::fs::write(PASSWD_PATH, PASSWD_MODE, db.serialize().as_bytes());
}

pub fn run() -> Session {
    let mut db = load_accounts();

    let session = greeter::run(&mut KernelPlatform, &mut db);

    if db.dirty {
        save_accounts(&db);
    }
    session
}