use crate::session::Session;
use crate::user_accounts::UserInfo;
use crate::{border, centered, fill, text, Color, Key, Platform};
use alloc::{format, string::String, vec::Vec};

const W: usize = 64;
const H: usize = 17;
const LIST_ROWS: usize = 9;
const MAX_ATTEMPTS: u32 = 3;

fn panel<P: Platform>(p: &mut P) -> (usize, usize) {
    let (sw, sh) = p.size();
    let (ox, oy) = (sw.saturating_sub(W) / 2, sh.saturating_sub(H) / 2);
    p.clear(Color::Blue);
    fill(p, ox, oy, W, H, Color::LightGray);
    border(p, ox, oy, W, H, Color::Black, Color::LightGray);
    (ox, oy)
}

fn draw<P: Platform>(
    p: &mut P,
    users: &[UserInfo],
    sel: usize,
    pw_len: usize,
    msg: &str,
    pressed: bool,
) {
    let (ox, oy) = panel(p);
    centered(p, ox, W, oy + 1, "ZodiacOS - Sign in", Color::Black, Color::LightGray);

    // ---- users list ----
    let (lx, ly, lw) = (ox + 2, oy + 3, 22);
    border(p, lx, ly, lw, LIST_ROWS + 2, Color::Black, Color::LightGray);
    text(p, lx + 2, ly, " Users ", Color::Black, Color::LightGray);
    let start = if sel >= LIST_ROWS { sel + 1 - LIST_ROWS } else { 0 };
    for (row, u) in users.iter().enumerate().skip(start).take(LIST_ROWS) {
        let y = ly + 1 + (row - start);
        let name: String = u.name.chars().take(lw - 6).collect();
        if row == sel {
            fill(p, lx + 1, y, lw - 2, 1, Color::Blue);
            text(p, lx + 2, y, &format!("> {}", name), Color::White, Color::Blue);
        } else {
            text(p, lx + 2, y, &format!("  {}", name), Color::Black, Color::LightGray);
        }
    }

    // ---- profile ----
    let rx = ox + 27;
    let user = &users[sel];
    fill(p, rx, oy + 3, 9, 5, Color::Cyan);
    border(p, rx, oy + 3, 9, 5, Color::Black, Color::Cyan);
    let initial = user.name.chars().next().map(|c| c.to_ascii_uppercase()).unwrap_or('?');
    p.put_char(rx + 4, oy + 5, initial, Color::White, Color::Cyan);
    text(p, rx + 11, oy + 4, &user.name, Color::Black, Color::LightGray);
    text(p, rx + 11, oy + 6, &format!("uid: {}", user.uid), Color::DarkGray, Color::LightGray);

    // ---- password box ----
    text(p, rx, oy + 9, "Password:", Color::Black, Color::LightGray);
    border(p, rx, oy + 10, 35, 3, Color::Black, Color::LightGray);
    fill(p, rx + 1, oy + 11, 33, 1, Color::White);
    let shown = pw_len.min(31);
    let mut stars = String::new();
    for _ in 0..shown {
        stars.push('*');
    }
    stars.push('_');
    text(p, rx + 1, oy + 11, &stars, Color::Black, Color::White);

    // ---- login button ----
    let (fg, bg) = if pressed { (Color::Yellow, Color::Blue) } else { (Color::Black, Color::Green) };
    text(p, rx, oy + 13, "[  Login  ]", fg, bg);

    text(p, ox + 2, oy + 14, "Tab: next user   Enter: login", Color::DarkGray, Color::LightGray);
    text(p, ox + 2, oy + 15, msg, Color::Red, Color::LightGray);
}

/// Reads a hidden line inside a one-line box. Returns the typed text.
fn read_secret<P: Platform>(p: &mut P, x: usize, y: usize, w: usize) -> String {
    let mut s = String::new();
    loop {
        fill(p, x, y, w, 1, Color::White);
        let mut stars = String::new();
        for _ in 0..s.chars().count().min(w - 2) {
            stars.push('*');
        }
        stars.push('_');
        text(p, x, y, &stars, Color::Black, Color::White);

        match p.read_key() {
            Key::Enter => return s,
            Key::Backspace => {
                s.pop();
            }
            Key::Char(c) if s.len() < 64 => s.push(c),
            _ => {}
        }
    }
}

/// Fresh install: ask for the root password. The kernel does the actual work.
fn setup_root<P: Platform>(p: &mut P) {
    let mut msg = "";
    loop {
        let (ox, oy) = panel(p);
        centered(p, ox, W, oy + 2, "First boot: create the root account", Color::Black, Color::LightGray);
        text(p, ox + 10, oy + 5, "New password:", Color::Black, Color::LightGray);
        text(p, ox + 10, oy + 9, "Repeat password:", Color::Black, Color::LightGray);
        text(p, ox + 10, oy + 14, msg, Color::Red, Color::LightGray);
        border(p, ox + 10, oy + 6, 40, 3, Color::Black, Color::LightGray);
        border(p, ox + 10, oy + 10, 40, 3, Color::Black, Color::LightGray);

        let a = read_secret(p, ox + 11, oy + 7, 38);
        let b = read_secret(p, ox + 11, oy + 11, 38);

        if a != b {
            msg = "Passwords do not match.";
        } else if p.create_root(&a).is_err() {
            msg = "Password rejected (empty?).";
        } else {
            return;
        }
    }
}

/// Shows the sign-in screen until the kernel accepts a login.
pub fn run<P: Platform>(p: &mut P) -> Session {
    if !p.has_accounts() {
        setup_root(p);
    }
    let users: Vec<UserInfo> = p.list_users();
    let mut sel = 0usize;
    let mut pw = String::new();
    let mut msg = "";
    let mut failures = 0u32;

    loop {
        draw(p, &users, sel, pw.chars().count(), msg, false);

        match p.read_key() {
            Key::Enter => {
                draw(p, &users, sel, pw.chars().count(), msg, true);
                p.sleep_ms(120);
                match p.authenticate(&users[sel].name, &pw) {
                    Ok(session) => return session,
                    Err(_) => {
                        pw.clear();
                        failures += 1;
                        if failures >= MAX_ATTEMPTS {
                            draw(p, &users, sel, 0, "Too many failures, wait...", false);
                            p.sleep_ms(3000);
                            failures = 0;
                            msg = "";
                        } else {
                            msg = "Incorrect password.";
                            p.sleep_ms(800);
                        }
                    }
                }
            }
            Key::Backspace => {
                pw.pop();
            }
            Key::Tab | Key::Down => {
                sel = (sel + 1) % users.len();
                pw.clear();
                msg = "";
            }
            Key::Up => {
                sel = (sel + users.len() - 1) % users.len();
                pw.clear();
                msg = "";
            }
            Key::Char(c) if pw.len() < 64 => pw.push(c),
            _ => {}
        }
    }
}