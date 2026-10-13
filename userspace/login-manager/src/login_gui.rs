//! Pixel-based login screen for a 1920x1080 framebuffer, drawn through `Gfx`,
//! with Roboto text (see typeface.rs).
//!
//! The wallpaper and card frame are painted ONCE. After that only small opaque
//! regions (identity block, form block) are cleared and redrawn, so there is no
//! full-screen flicker and no alpha build-up.

use crate::components::Input;
use crate::session::Session;
use crate::typeface::Ui;
use crate::user_accounts::UserInfo;
use crate::{Gfx, Key, Platform};
use alloc::{format, string::String, vec::Vec};

// ---- palette (0xRRGGBB) ------------------------------------------------------
const BG_TOP: u32 = 0x0B1020;
const BG_BOT: u32 = 0x1B1236;
const CARD: u32 = 0x161828;
const CARD_BORDER: u32 = 0x2A2D4A;
const FIELD_BG: u32 = 0x0E0F1C;
const FIELD_BORDER: u32 = 0x34375A;
const ACCENT: u32 = 0x6366F1;
const BTN_OFF: u32 = 0x24264A;
const CHIP: u32 = 0x23253F;
const TEXT: u32 = 0xF1F5F9;
const MUTED: u32 = 0x8B90A8;
const DIM: u32 = 0x5C6080;
const ERR: u32 = 0xF87171;

// ---- layout ------------------------------------------------------------------
const CARD_W: usize = 460;
const CARD_H: usize = 520;
const CARD_R: usize = 28;
const FIELD_W: usize = 380;
const FIELD_H: usize = 56;
const DOT_PITCH: usize = 24;
const MAX_DOTS: usize = 13;
const MAX_ATTEMPTS: u32 = 3;
const INPUT_CAP: usize = 64;

struct Geo {
    sw: usize,
    sh: usize,
    cx: usize,
    card_x: usize,
    card_y: usize,
}

impl Geo {
    fn new(sw: usize, sh: usize) -> Self {
        let cx = sw / 2;
        Geo { sw, sh, cx, card_x: cx.saturating_sub(CARD_W / 2), card_y: sh * 31 / 100 }
    }
}

/// The pixel canvas of the platform (run_gui is only entered when it exists).
fn gx<P: Platform>(p: &mut P) -> &mut dyn Gfx {
    p.gfx().expect("login_gui needs Platform::gfx()")
}

/// TODO: replace with a real RTC read (drivers/rtc.rs).
fn wall_time() -> (u8, u8) {
    (9, 41)
}

// ---- static pieces (drawn once) ------------------------------------------------

fn draw_wallpaper(c: &mut dyn Gfx, g: &Geo) {
    c.gradient(0, 0, g.sw, g.sh, BG_TOP, BG_BOT);
    // soft "aurora" blobs
    c.circle(g.sw * 20 / 100, g.sh * 25 / 100, 420, 0x6366F1, 38);
    c.circle(g.sw * 85 / 100, g.sh * 80 / 100, 520, 0xEC4899, 26);
    c.circle(g.sw * 70 / 100, g.sh * 15 / 100, 260, 0x22D3EE, 20);

    // card shadow: a few expanding, very faint black layers
    for i in 1..=6usize {
        let e = i * 5;
        c.round_rect(
            g.card_x.saturating_sub(e),
            (g.card_y + 16).saturating_sub(e),
            CARD_W + 2 * e,
            CARD_H + 2 * e,
            CARD_R + e,
            0x000000,
            14,
        );
    }
}

fn draw_clock(c: &mut dyn Gfx, ui: &mut Ui, g: &Geo) {
    let (h, m) = wall_time();
    let s = format!("{:02}:{:02}", h, m);
    ui.centered(c, g.cx, g.sh / 12 + 60, &s, 150, false, TEXT);
}

fn draw_card(c: &mut dyn Gfx, g: &Geo) {
    c.round_rect(g.card_x, g.card_y, CARD_W, CARD_H, CARD_R, CARD_BORDER, 255);
    c.round_rect(g.card_x + 1, g.card_y + 1, CARD_W - 2, CARD_H - 2, CARD_R - 1, CARD, 255);
}

// ---- identity block (avatar + name), redrawn on user switch ---------------------

fn draw_identity(
    c: &mut dyn Gfx,
    ui: &mut Ui,
    g: &Geo,
    initial: char,
    title: &str,
    subtitle: &str,
) {
    c.fill(g.card_x + 16, g.card_y + 30, CARD_W - 32, 196, CARD);
    let cy = g.card_y + 96;
    c.circle(g.cx, cy, 56, ACCENT, 56); // glow
    c.circle(g.cx, cy, 46, ACCENT, 255);

    let initial = if initial.is_ascii() { initial } else { '?' };
    let mut buf = [0u8; 4];
    let s = initial.encode_utf8(&mut buf);
    ui.centered(c, g.cx, cy, s, 46, true, 0xFFFFFF);

    ui.centered(c, g.cx, g.card_y + 178, title, 28, true, TEXT);
    ui.centered(c, g.cx, g.card_y + 210, subtitle, 16, false, MUTED);
}

// ---- form widgets ----------------------------------------------------------------

fn clear_form(c: &mut dyn Gfx, g: &Geo) {
    c.fill(g.card_x + 16, g.card_y + 226, CARD_W - 32, 262, CARD);
}

/// Web-style input: rounded border, focus glow, placeholder, dot password, caret.
fn draw_field(
    c: &mut dyn Gfx,
    ui: &mut Ui,
    g: &Geo,
    y: usize,
    dx: isize,
    n: usize,
    focused: bool,
    error: bool,
    placeholder: &str,
) {
    let x = (g.cx as isize - (FIELD_W / 2) as isize + dx).max(0) as usize;

    if error {
        c.round_rect(x.saturating_sub(4), y - 4, FIELD_W + 8, FIELD_H + 8, 18, ERR, 40);
    } else if focused {
        c.round_rect(x.saturating_sub(4), y - 4, FIELD_W + 8, FIELD_H + 8, 18, ACCENT, 48);
    }
    let border = if error { ERR } else if focused { ACCENT } else { FIELD_BORDER };
    c.round_rect(x, y, FIELD_W, FIELD_H, 14, border, 255);
    c.round_rect(x + 2, y + 2, FIELD_W - 4, FIELD_H - 4, 12, FIELD_BG, 255);

    if n == 0 {
        ui.text(c, x + 22, y + FIELD_H / 2, placeholder, 19, false, DIM);
        if focused {
            c.fill(x + 14, y + 14, 2, 28, ACCENT); // caret
        }
    } else {
        let shown = n.min(MAX_DOTS);
        for i in 0..shown {
            c.circle(x + 28 + i * DOT_PITCH, y + FIELD_H / 2, 7, TEXT, 255);
        }
        if focused {
            c.fill(x + 18 + shown * DOT_PITCH, y + 14, 2, 28, ACCENT);
        }
    }
}

fn draw_button(c: &mut dyn Gfx, ui: &mut Ui, g: &Geo, y: usize, enabled: bool, label: &str) {
    let x = g.cx - FIELD_W / 2;
    let (bg, fg) = if enabled { (ACCENT, 0xFFFFFF) } else { (BTN_OFF, MUTED) };
    c.round_rect(x, y, FIELD_W, FIELD_H, 14, bg, 255);
    ui.centered(c, g.cx, y + FIELD_H / 2, label, 19, true, fg);
}

fn draw_hint(c: &mut dyn Gfx, ui: &mut Ui, g: &Geo, s: &str) {
    ui.centered(c, g.cx, g.card_y + 470, s, 13, false, DIM);
}

fn draw_login_form(
    c: &mut dyn Gfx,
    ui: &mut Ui,
    g: &Geo,
    n: usize,
    msg: &str,
    dx: isize,
    error: bool,
) {
    clear_form(c, g);
    draw_field(c, ui, g, g.card_y + 260, dx, n, true, error, "Password");
    if !msg.is_empty() {
        ui.centered(c, g.cx, g.card_y + 340, msg, 15, false, if error { ERR } else { MUTED });
    }
    draw_button(c, ui, g, g.card_y + 380, n > 0, "Sign in");
    draw_hint(c, ui, g, "Tab: switch user    Enter: sign in");
}

fn draw_chips(c: &mut dyn Gfx, ui: &mut Ui, g: &Geo, users: &[UserInfo], sel: usize) {
    if users.len() < 2 {
        return;
    }
    let names: Vec<String> = users.iter().map(|u| u.name.chars().take(12).collect()).collect();
    let widths: Vec<usize> = names.iter().map(|n| ui.width(c, n, 15, false) + 44).collect();
    let total: usize = widths.iter().sum::<usize>() + 12 * (names.len() - 1);
    let mut x = g.cx.saturating_sub(total / 2);
    let y = g.card_y + CARD_H + 36;

    for (i, name) in names.iter().enumerate() {
        let (bg, fg) = if i == sel { (ACCENT, 0xFFFFFF) } else { (CHIP, MUTED) };
        c.round_rect(x, y, widths[i], 40, 20, bg, 255);
        ui.text(c, x + 22, y + 20, name, 15, false, fg);
        x += widths[i] + 12;
    }
}

fn show_user(c: &mut dyn Gfx, ui: &mut Ui, g: &Geo, users: &[UserInfo], sel: usize) {
    let name = &users[sel].name;
    let initial = name.chars().next().map(|ch| ch.to_ascii_uppercase()).unwrap_or('?');
    draw_identity(c, ui, g, initial, name, "Enter your password");
    draw_chips(c, ui, g, users, sel);
}

// ---- first-boot setup -------------------------------------------------------------

fn draw_setup_form(
    c: &mut dyn Gfx,
    ui: &mut Ui,
    g: &Geo,
    a: usize,
    b: usize,
    focus: usize,
    msg: &str,
) {
    clear_form(c, g);
    draw_field(c, ui, g, g.card_y + 240, 0, a, focus == 0, false, "New password");
    draw_field(c, ui, g, g.card_y + 308, 0, b, focus == 1, false, "Verify password");
    if !msg.is_empty() {
        ui.centered(c, g.cx, g.card_y + 378, msg, 15, false, ERR);
    }
    draw_button(c, ui, g, g.card_y + 400, b > 0, "Create account");
    draw_hint(c, ui, g, "First boot    Enter: next / confirm");
}

fn setup_root<P: Platform>(p: &mut P, ui: &mut Ui, g: &Geo) {
    draw_identity(gx(p), ui, g, 'R', "Create root account", "Choose a password for first boot");
    let mut a = Input::new(0, 0, INPUT_CAP, true);
    let mut b = Input::new(0, 0, INPUT_CAP, true);
    let mut focus = 0usize;
    let mut msg: &str = "";

    loop {
        let (na, nb) = (a.as_str().chars().count(), b.as_str().chars().count());
        draw_setup_form(gx(p), ui, g, na, nb, focus, msg);

        match p.read_key() {
            Key::Enter => {
                if focus == 0 {
                    if a.as_str().is_empty() {
                        msg = "Enter a password.";
                    } else {
                        msg = "";
                        focus = 1;
                    }
                } else if a.as_str() != b.as_str() {
                    msg = "Passwords do not match.";
                    b.clear();
                } else if p.create_root(a.as_str()).is_err() {
                    msg = "Password rejected.";
                    a.clear();
                    b.clear();
                    focus = 0;
                } else {
                    return;
                }
            }
            Key::Tab => focus = 1 - focus,
            Key::Backspace => {
                msg = "";
                if focus == 0 { a.backspace() } else { b.backspace() }
            }
            Key::Char(ch) => {
                msg = "";
                if focus == 0 { a.push_char(ch) } else { b.push_char(ch) }
            }
            _ => {}
        }
    }
}

// ---- entry point ---------------------------------------------------------------------

pub fn run_gui<P: Platform>(p: &mut P) -> Session {
    let (sw, sh) = gx(p).size_px();
    let g = Geo::new(sw, sh);
    let mut ui = Ui::new();

    {
        let c = gx(p);
        draw_wallpaper(c, &g);
        draw_clock(c, &mut ui, &g);
        draw_card(c, &g);
    }

    if !p.has_accounts() {
        setup_root(p, &mut ui, &g);
    }

    let users: Vec<UserInfo> = p.list_users();
    let mut sel = 0usize;
    let mut password = Input::new(0, 0, INPUT_CAP, true);
    let mut msg: &str = "";
    let mut err = false;
    let mut failures = 0u32;

    show_user(gx(p), &mut ui, &g, &users, sel);

    loop {
        draw_login_form(gx(p), &mut ui, &g, password.as_str().chars().count(), msg, 0, err);

        match p.read_key() {
            Key::Enter => {
                if password.as_str().is_empty() {
                    continue;
                }
                match p.authenticate(&users[sel].name, password.as_str()) {
                    Ok(session) => return session,
                    Err(_) => {
                        password.clear();
                        failures += 1;
                        err = true;
                        if failures >= MAX_ATTEMPTS {
                            msg = "Too many attempts. Wait 3 seconds.";
                            draw_login_form(gx(p), &mut ui, &g, 0, msg, 0, true);
                            p.sleep_ms(3000);
                            failures = 0;
                            msg = "";
                            err = false;
                        } else {
                            msg = "Incorrect password";
                            for dx in [-12isize, 12, -8, 8, -4, 4, 0] {
                                draw_login_form(gx(p), &mut ui, &g, 0, msg, dx, true);
                                p.sleep_ms(35);
                            }
                        }
                    }
                }
            }
            Key::Backspace => {
                password.backspace();
                msg = "";
                err = false;
            }
            Key::Tab | Key::Down => {
                sel = (sel + 1) % users.len();
                password.clear();
                msg = "";
                err = false;
                show_user(gx(p), &mut ui, &g, &users, sel);
            }
            Key::Up => {
                sel = (sel + users.len() - 1) % users.len();
                password.clear();
                msg = "";
                err = false;
                show_user(gx(p), &mut ui, &g, &users, sel);
            }
            Key::Char(ch) => {
                password.push_char(ch);
                msg = "";
                err = false;
            }
            _ => {}
        }
    }
}