use crate::components::Input;
use crate::session::Session;
use crate::user_accounts::UserInfo;
use crate::{centered, fill, text, Color, Key, Platform};
use alloc::{format, string::String, vec::Vec};

const PILL_W: usize = 28;
const AV_W: usize = 11;
const AV_H: usize = 5;
const MAX_ATTEMPTS: u32 = 3;

// Clock scale: each font "pixel" is DIGIT_SX cols x DIGIT_SY rows.
const DIGIT_SX: usize = 3;
const DIGIT_SY: usize = 2;
const DIGIT_GAP: usize = 2;

// 3x5 digit font, one row per byte (bits 0b100 = left column).
const DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111], // 0
    [0b010, 0b110, 0b010, 0b010, 0b111], // 1
    [0b111, 0b001, 0b111, 0b100, 0b111], // 2
    [0b111, 0b001, 0b111, 0b001, 0b111], // 3
    [0b101, 0b101, 0b111, 0b001, 0b001], // 4
    [0b111, 0b100, 0b111, 0b001, 0b111], // 5
    [0b111, 0b100, 0b111, 0b101, 0b111], // 6
    [0b111, 0b001, 0b001, 0b001, 0b001], // 7
    [0b111, 0b101, 0b111, 0b101, 0b111], // 8
    [0b111, 0b101, 0b111, 0b001, 0b111], // 9
];

// ---------- wallpaper helpers ----------

/// Background colour for a given row: blue sky on top, black below.
fn bg(y: usize, sh: usize) -> Color {
    if y * 100 < sh * 45 { Color::Blue } else { Color::Black }
}

fn paint<P: Platform>(p: &mut P, sw: usize, sh: usize) {
    for y in 0..sh {
        fill(p, 0, y, sw, 1, bg(y, sh));
    }
}

/// text() that automatically uses the wallpaper colour behind it.
fn t<P: Platform>(p: &mut P, x: usize, y: usize, s: &str, fg: Color, sh: usize) {
    text(p, x, y, s, fg, bg(y, sh));
}

fn c<P: Platform>(p: &mut P, sw: usize, y: usize, s: &str, fg: Color, sh: usize) {
    centered(p, 0, sw, y, s, fg, bg(y, sh));
}

// ---------- clock ----------

/// TODO: replace with a real RTC read (CMOS ports 0x70/0x71 on x86).
fn wall_time<P: Platform>(_p: &P) -> (u8, u8) {
    (9, 41)
}

fn draw_digit<P: Platform>(p: &mut P, x: usize, y: usize, d: u8) {
    let rows = &DIGITS[(d % 10) as usize];
    for (r, bits) in rows.iter().enumerate() {
        for col in 0..3 {
            if bits & (0b100 >> col) != 0 {
                fill(p, x + col * DIGIT_SX, y + r * DIGIT_SY, DIGIT_SX, DIGIT_SY, Color::White);
            }
        }
    }
}

fn draw_clock<P: Platform>(p: &mut P, sw: usize, y: usize, h: u8, m: u8) {
    let dw = 3 * DIGIT_SX;
    let total = 4 * dw + DIGIT_SX + 4 * DIGIT_GAP;
    let mut x = sw.saturating_sub(total) / 2;

    draw_digit(p, x, y, h / 10);
    x += dw + DIGIT_GAP;
    draw_digit(p, x, y, h % 10);
    x += dw + DIGIT_GAP;

    // colon
    fill(p, x, y + DIGIT_SY, DIGIT_SX, DIGIT_SY, Color::White);
    fill(p, x, y + 3 * DIGIT_SY, DIGIT_SX, DIGIT_SY, Color::White);
    x += DIGIT_SX + DIGIT_GAP;

    draw_digit(p, x, y, m / 10);
    x += dw + DIGIT_GAP;
    draw_digit(p, x, y, m % 10);
}

// ---------- avatar + password pill ----------

fn draw_avatar<P: Platform>(p: &mut P, sw: usize, y: usize, initial: char) {
    let x = sw.saturating_sub(AV_W) / 2;
    for r in 0..AV_H {
        // clip the corners to fake a circle
        let (ax, aw) = if r == 0 || r == AV_H - 1 {
            (x + 2, AV_W - 4)
        } else {
            (x, AV_W)
        };
        fill(p, ax, y + r, aw, 1, Color::LightGray);
    }
    p.put_char(x + AV_W / 2, y + AV_H / 2, initial, Color::Black, Color::LightGray);
}

fn draw_pill<P: Platform>(
    p: &mut P,
    x: usize,
    y: usize,
    input: &Input,
    placeholder: &str,
    sh: usize,
) {
    let outer = bg(y, sh);
    fill(p, x + 1, y, PILL_W - 2, 1, Color::DarkGray);
    p.put_char(x, y, '▐', Color::DarkGray, outer);
    p.put_char(x + PILL_W - 1, y, '▌', Color::DarkGray, outer);

    let n = input.as_str().chars().count();
    if n == 0 {
        text(p, x + 3, y, placeholder, Color::LightGray, Color::DarkGray);
    } else {
        let mut dots = String::new();
        for _ in 0..n.min(PILL_W - 10) {
            dots.push('•');
        }
        dots.push('▌');
        text(p, x + 3, y, &dots, Color::White, Color::DarkGray);
        // submit arrow, like macOS
        text(p, x + PILL_W - 5, y, " → ", Color::Black, Color::White);
    }
}

fn draw_user_chips<P: Platform>(p: &mut P, sw: usize, sh: usize, users: &[UserInfo], sel: usize) {
    if users.len() < 2 {
        return;
    }
    let labels: Vec<String> = users
        .iter()
        .map(|u| {
            let ini = u.name.chars().next().map(|c| c.to_ascii_uppercase()).unwrap_or('?');
            let name: String = u.name.chars().take(12).collect();
            format!(" {}  {} ", ini, name)
        })
        .collect();

    let total: usize = labels.iter().map(|l| l.chars().count()).sum::<usize>() + 2 * (labels.len() - 1);
    let mut x = sw.saturating_sub(total) / 2;
    let y = sh.saturating_sub(4);

    for (i, label) in labels.iter().enumerate() {
        let (fg, bgc) = if i == sel {
            (Color::White, Color::DarkGray)
        } else {
            (Color::LightGray, Color::Black)
        };
        text(p, x, y, label, fg, bgc);
        x += label.chars().count() + 2;
    }
}

// ---------- screens ----------

fn draw_login<P: Platform>(
    p: &mut P,
    users: &[UserInfo],
    sel: usize,
    password: &Input,
    msg: &str,
    dx: isize, // horizontal offset for the shake animation
) {
    let (sw, sh) = p.size();
    paint(p, sw, sh);

    let top = (sh / 10).max(2);
    let (h, m) = wall_time(p);
    draw_clock(p, sw, top, h, m);

    let ay = sh * 48 / 100;
    let user = &users[sel];
    let initial = user.name.chars().next().map(|c| c.to_ascii_uppercase()).unwrap_or('?');

    draw_avatar(p, sw, ay, initial);
    c(p, sw, ay + AV_H + 1, &user.name, Color::White, sh);

    let px = (sw.saturating_sub(PILL_W) / 2) as isize + dx;
    draw_pill(p, px.max(0) as usize, ay + AV_H + 3, password, "Enter Password", sh);

    if !msg.is_empty() {
        c(p, sw, ay + AV_H + 5, msg, Color::White, sh);
    }

    draw_user_chips(p, sw, sh, users, sel);
    c(p, sw, sh.saturating_sub(2), "Tab / ↑ ↓  switch user   ·   Enter  log in", Color::LightGray, sh);
}

fn draw_setup<P: Platform>(p: &mut P, a: &Input, b: &Input, msg: &str) {
    let (sw, sh) = p.size();
    paint(p, sw, sh);

    let top = (sh / 10).max(2);
    let (h, m) = wall_time(p);
    draw_clock(p, sw, top, h, m);

    let ay = sh * 48 / 100;
    draw_avatar(p, sw, ay, 'R');
    c(p, sw, ay + AV_H + 1, "Create root account", Color::White, sh);

    let px = sw.saturating_sub(PILL_W) / 2;
    draw_pill(p, px, ay + AV_H + 3, a, "New password", sh);
    draw_pill(p, px, ay + AV_H + 5, b, "Verify password", sh);

    if !msg.is_empty() {
        c(p, sw, ay + AV_H + 7, msg, Color::White, sh);
    }
    c(p, sw, sh.saturating_sub(2), "First boot  ·  Enter  next / confirm", Color::LightGray, sh);
}

fn shake<P: Platform>(p: &mut P, users: &[UserInfo], sel: usize, password: &Input, msg: &str) {
    for dx in [-3isize, 3, -2, 2, -1, 1, 0] {
        draw_login(p, users, sel, password, msg, dx);
        p.sleep_ms(40);
    }
}

fn setup_root<P: Platform>(p: &mut P) {
    let mut msg = "";
    let mut a = Input::new(0, 0, PILL_W, true);
    let mut b = Input::new(0, 0, PILL_W, true);

    loop {
        a.clear();
        b.clear();

        loop {
            draw_setup(p, &a, &b, msg);
            match p.read_key() {
                Key::Enter => break,
                Key::Backspace => a.backspace(),
                Key::Char(ch) => a.push_char(ch),
                _ => {}
            }
        }
        loop {
            draw_setup(p, &a, &b, msg);
            match p.read_key() {
                Key::Enter => break,
                Key::Backspace => b.backspace(),
                Key::Char(ch) => b.push_char(ch),
                _ => {}
            }
        }

        if a.as_str() != b.as_str() {
            msg = "Passwords do not match.";
        } else if p.create_root(a.as_str()).is_err() {
            msg = "Password rejected (empty?).";
        } else {
            return;
        }
    }
}

pub fn run<P: Platform>(p: &mut P) -> Session {
    if !p.has_accounts() {
        setup_root(p);
    }

    let users: Vec<UserInfo> = p.list_users();
    let mut sel = 0usize;
    let mut password = Input::new(0, 0, PILL_W, true);
    let mut msg = "";
    let mut failures = 0u32;

    loop {
        draw_login(p, &users, sel, &password, msg, 0);

        match p.read_key() {
            Key::Enter => match p.authenticate(&users[sel].name, password.as_str()) {
                Ok(session) => return session,
                Err(_) => {
                    password.clear();
                    failures += 1;
                    if failures >= MAX_ATTEMPTS {
                        msg = "Too many attempts. Please wait…";
                        draw_login(p, &users, sel, &password, msg, 0);
                        p.sleep_ms(3000);
                        failures = 0;
                        msg = "";
                    } else {
                        msg = "Incorrect password";
                        shake(p, &users, sel, &password, msg);
                    }
                }
            },
            Key::Backspace => password.backspace(),
            Key::Tab | Key::Down => {
                sel = (sel + 1) % users.len();
                password.clear();
                msg = "";
            }
            Key::Up => {
                sel = (sel + users.len() - 1) % users.len();
                password.clear();
                msg = "";
            }
            Key::Char(ch) => password.push_char(ch),
            _ => {}
        }
    }
}