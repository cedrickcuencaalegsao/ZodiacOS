#![allow(dead_code)]
use super::{auth::AuthError, service};
use crate::console::prompt::{read_line, read_password};
use crate::kprintln;

/// Runs first-time setup if needed, then blocks until someone logs in.
pub fn run(now: impl Fn() -> u64) {
    service::init(None); // later: accounts file read from ZodiacFS

    if service::needs_setup() {
        kprintln!("First-time setup: choose a root password (8+ characters).");
        loop {
            let pw = read_password("New root password: ");
            let again = read_password("Confirm password:  ");
            if pw != again {
                kprintln!("Passwords do not match.\n");
                continue;
            }
            match service::first_run_setup(&pw) {
                Ok(code) => {
                    kprintln!("\nRecovery code (write it down, shown once):");
                    kprintln!("    {code}\n");
                    break;
                }
                Err(AuthError::WeakPassword) => kprintln!("Too short.\n"),
                Err(AuthError::NoEntropy) => {
                    // No RDRAND: refuse rather than create weak salts.
                    panic!("no hardware RNG; run QEMU with -cpu max");
                }
                Err(e) => kprintln!("Setup failed: {:?}\n", e),
            }
        }
    }

    loop {
        let name = read_line("login: ");
        let pw = read_password("password: ");
        match service::login(&name, &pw, now()) {
            Ok(_) => return,
            Err(_) => kprintln!("Login incorrect.\n"), // same message for unknown user or bad password
        }
    }
}