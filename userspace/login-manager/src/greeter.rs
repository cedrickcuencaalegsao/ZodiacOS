use crate::authentication::{authenticate, make_account};
use crate::session::Session;
use crate::user_accounts::AccountDb;
use crate::Platform;

const MAX_ATTEMPTS: u32 = 3;
const LOCKOUT_MS: u64 = 3000;

/// Runs until someone logs in successfully. Never returns without a Session.
pub fn run<P: Platform>(p: &mut P, db: &mut AccountDb) -> Session {
    if db.is_empty() {
        first_boot_setup(p, db);
    }

    let mut failures = 0;
    loop {
        p.clear();
        p.write_str("ZodiacOS 0.1.0\n\n");

        p.write_str("login: ");
        let name = p.read_line(true);
        let name = name.trim();
        if name.is_empty() {
            continue;
        }

        p.write_str("Password: ");
        let password = p.read_line(false);
        p.write_str("\n");

        match authenticate(db, name, &password) {
            Ok(session) => {
                p.write_str("\nWelcome, ");
                p.write_str(&session.name);
                p.write_str("!\n\n");
                return session;
            }
            Err(_) => {
                failures += 1;
                p.write_str("Login incorrect.\n");
                if failures >= MAX_ATTEMPTS {
                    p.write_str("Too many failures, please wait...\n");
                    p.sleep_ms(LOCKOUT_MS);
                    failures = 0;
                } else {
                    p.sleep_ms(1000);
                }
            }
        }
    }
}

/// No accounts exist yet (fresh install): create the root account.
fn first_boot_setup<P: Platform>(p: &mut P, db: &mut AccountDb) {
    p.clear();
    p.write_str("First boot: create the root account.\n\n");

    loop {
        p.write_str("New root password: ");
        let a = p.read_line(false);
        p.write_str("\nRepeat password:   ");
        let b = p.read_line(false);
        p.write_str("\n");

        if a.is_empty() {
            p.write_str("Password cannot be empty.\n\n");
        } else if a != b {
            p.write_str("Passwords do not match.\n\n");
        } else {
            let mut salt = [0u8; 16];
            p.random_bytes(&mut salt);
            db.add(make_account("root", &a, 0, 0, salt));
            p.write_str("Root account created.\n");
            p.sleep_ms(800);
            return;
        }
    }
}