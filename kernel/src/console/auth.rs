use crate::interrupts::{sleep_ms, ticks};
use crate::security::service;
use crate::security::session::Session;
use crate::{keyboard, kprint, kprintln, log_info, log_warn, vga};
use alloc::string::String;
use spin::Mutex;

static PENDING_RECOVERY: Mutex<Option<String>> = Mutex::new(None); // shown once by lib.rs

pub fn take_recovery_code() -> Option<String> {
    PENDING_RECOVERY.lock().take()
}

fn first_run() {
    kprintln!("Welcome to ZodiacOS. No accounts exist yet.");
    kprintln!("Set a password for the root account.\n");
    loop {
        kprint!("New root password: ");
        let a = read_line(false);
        kprint!("Confirm password: ");
        let b = read_line(false);
        if a != b {
            kprintln!("Passwords do not match.\n");
            continue;
        }
        match service::first_run_setup(&a) {
            Ok(code) => {
                *PENDING_RECOVERY.lock() = Some(code);
                flush_accounts();
                kprintln!("Root account created.");
                return;
            }
            Err(e) => kprintln!("Could not create account: {:?}\n", e),
        }
    }
}

fn recover_flow() {
    kprint!("username: ");
    let name = read_line(true);
    kprint!("recovery code: ");
    let code = read_line(false);
    kprint!("new password: ");
    let pw = read_line(false);
    match service::recover(&name, &code, &pw) {
        Ok(new_code) => {
            flush_accounts();
            kprintln!(
                "Password reset. NEW recovery code (write it down): {}",
                new_code
            );
        }
        Err(_) => kprintln!("Recovery failed."),
    }
}

/// Persist accounts to the drive whenever they changed.
/// Call after any service mutation: first_run, add_user, passwd, recover.
pub fn flush_accounts() {
    if let Some(data) = service::take_pending_save() {
        // ADAPT: write to ZodiacFS, e.g. System/Config/accounts
        // if crate::filesystem::write_system_item("accounts", data.as_bytes()).is_err() { ... }
        let _ = data;
    }
}
fn failure_delay() {
    sleep_ms(1000);
}

pub fn read_line(echo: bool) -> String {
    let mut s = String::new();
    loop {
        let b = keyboard::wait_char(); // u8
        match b {
            b'\n' | b'\r' => {
                kprintln!();
                return s;
            }
            0x08 | 0x7f => {
                if s.pop().is_some() && echo {
                    kprint!("\u{8} \u{8}");
                }
            }
            0x20..=0x7e if s.len() < 128 => {
                let c = b as char;
                s.push(c);
                if echo {
                    kprint!("{}", c);
                }
            }
            _ => {}
        }
    }
}

fn valid_username(n: &str) -> bool {
    n != "recover"
        && !n.is_empty()
        && n.len() <= 32
        && n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// `adduser <name>`: root only. Prompts for a password twice.
pub fn adduser_cmd(name: Option<&str>) {
    let Some(name) = name else {
        kprintln!("usage: adduser <name>");
        return;
    };
    // use the same root check as the service
    match service::whoami() {
        Some(s) if s.is_root() => {}
        _ => {
            kprintln!("adduser: permission denied (root only)");
            return;
        }
    }
    if !valid_username(name) {
        kprintln!("adduser: invalid name (letters, digits, _ and - only, max 32)");
        return;
    }
    if service::list_users().iter().any(|u| u.name == name) {
        kprintln!("adduser: user '{}' already exists", name);
        return;
    }
    loop {
        kprint!("New password: ");
        let a = read_line(false);
        kprint!("Confirm password: ");
        let b = read_line(false);
        if a.is_empty() {
            kprintln!("Password cannot be empty.\n");
            continue;
        }
        if a != b {
            kprintln!("Passwords do not match.\n");
            continue;
        }
        match service::add_user(name, &a) {
            Ok(code) => {
                flush_accounts();
                kprintln!("User '{}' created.", name);
                kprintln!("Recovery code (give it to the user, shown once): {}", code);
            }
            Err(e) => kprintln!("adduser: {:?}", e),
        }
        return;
    }
}

enum Choice {
    User(String),
    Recover,
}

/// Lists every account; returns the chosen one (by number or name).
fn select_user() -> Choice {
    loop {
        let users = service::list_users();
        kprintln!("\nSelect an account:");
        for (i, u) in users.iter().enumerate() {
            kprintln!("  [{}] {}", i + 1, u.name);
        }
        kprintln!("  (number or name, or 'recover' to reset a password)");
        kprint!("> ");
        let input = read_line(true);
        let input = input.trim();

        if input == "recover" {
            return Choice::Recover;
        }
        if let Some(u) = users.iter().find(|u| u.name == input) {
            return Choice::User(u.name.clone());
        }
        if let Ok(n) = input.parse::<usize>() {
            if n >= 1 && n <= users.len() {
                return Choice::User(users[n - 1].name.clone());
            }
        }
        kprintln!("No such account.");
    }
}

/// Blocks until a user is logged in. Handles first-run setup and login.
pub fn run() -> Session {
    if service::needs_setup() {
        first_run();
    }
    loop {
        match select_user() {
            Choice::Recover => recover_flow(),
            Choice::User(name) => {
                kprint!("Password for {}: ", name);
                let pw = read_line(false);
                match service::login(&name, &pw, ticks()) {
                    Ok(_) => {
                        log_info!("login ok: {}", name);
                        return service::whoami().expect("session just opened");
                    }
                    Err(e) => {
                        kprintln!("Login incorrect.");
                        log_warn!("login failed for '{}': {:?}", name, e);
                        failure_delay();
                    }
                }
            }
        }
    }
}

/// `logout` / `login`: close every session (including `su` ones), then
/// show the account picker again. Blocks until someone logs in.
pub fn logout_cmd() {
    if let Some(s) = service::whoami() {
        log_info!("logout: {}", s.name);
    }
    service::logout_all();
    vga::clear();
    let s = run();
    vga::clear();
    kprintln!(
        "Logged in as {}. Type 'help' for a list of commands.\n",
        s.name
    );
}

/// `user <list|passwd>`: root only.
pub fn user_cmd(args: &[&str]) {
    match service::whoami() {
        Some(s) if s.is_root() => {}
        _ => {
            kprintln!("user: permission denied (root only)");
            return;
        }
    }
    match args.first().copied() {
        Some("list") => {
            let users = service::list_users();
            kprintln!("{} account(s):", users.len());
            for (i, u) in users.iter().enumerate() {
                kprintln!("  [{}] {}", i + 1, u.name);
            }
        }
        Some("passwd") => match args.get(1) {
            Some(name) => reset_password_cmd(name),
            None => kprintln!("usage: user passwd <name>"),
        },
        _ => kprintln!("usage: user <list|passwd <name>>"),
    }
}

fn reset_password_cmd(name: &str) {
    if !service::list_users().iter().any(|u| u.name == name) {
        kprintln!("user: no such account '{}'", name);
        return;
    }
    loop {
        kprint!("New password for {}: ", name);
        let a = read_line(false);
        kprint!("Confirm password: ");
        let b = read_line(false);
        if a.is_empty() || a != b {
            kprintln!("Passwords empty or not matching.\n");
            continue;
        }
        match service::reset_password(name, &a) {
            Ok(()) => {
                flush_accounts();
                kprintln!("Password for '{}' changed.", name);
            }
            Err(e) => kprintln!("user: {:?}", e),
        }
        return;
    }
}