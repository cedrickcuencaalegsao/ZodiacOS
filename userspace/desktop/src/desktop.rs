//! The desktop: windows, z-order, pointer handling and the main loop.
use crate::apps::{self, App};
use crate::gfx::{text_width, Gfx, BL, BR, TL, TR};
use crate::{dock, panel, wallpaper, Event, Host, Key};

const MAX_WIN: usize = 8;
const TITLE_H: i32 = 30;
const KEY_STEP: i32 = 14; // cursor speed when driven from the keyboard

#[derive(Clone, Copy)]
struct Window {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    app: App,
    open: bool,
}

const CLOSED: Window = Window { x: 0, y: 0, w: 0, h: 0, app: App::Welcome, open: false };

struct Desktop {
    sw: i32,
    sh: i32,
    wins: [Window; MAX_WIN],
    order: [usize; MAX_WIN], // open windows, back to front
    count: usize,
    mx: i32,
    my: i32,
    drag: Option<(usize, i32, i32)>, // (window, grab offset x, grab offset y)
    menu: bool,
    last_sec: u64,
    dirty: bool,
    quit: bool,
}

impl Desktop {
    fn new(sw: i32, sh: i32) -> Self {
        Desktop {
            sw,
            sh,
            wins: [CLOSED; MAX_WIN],
            order: [0; MAX_WIN],
            count: 0,
            mx: sw / 2,
            my: sh / 2,
            drag: None,
            menu: false,
            last_sec: u64::MAX,
            dirty: true,
            quit: false,
        }
    }

    /// Opens the app, or brings its window to the front if it is already open.
    fn open_app(&mut self, app: App) {
        if let Some(i) = (0..MAX_WIN).find(|&i| self.wins[i].open && self.wins[i].app == app) {
            self.raise(i);
            return;
        }
        if let Some(i) = (0..MAX_WIN).find(|&i| !self.wins[i].open) {
            let (w, h) = app.size();
            let off = self.count as i32 * 26;
            let x = ((self.sw - w) / 2 + off - 60).max(8);
            let y = ((self.sh - h) / 2 + off - 40).max(panel::HEIGHT + 8);
            self.wins[i] = Window { x, y, w, h, app, open: true };
            self.order[self.count] = i;
            self.count += 1;
        }
    }

    fn remove_from_order(&mut self, idx: usize) {
        if let Some(p) = self.order[..self.count].iter().position(|&i| i == idx) {
            for k in p..self.count - 1 {
                self.order[k] = self.order[k + 1];
            }
            self.count -= 1;
        }
    }

    fn raise(&mut self, idx: usize) {
        self.remove_from_order(idx);
        self.order[self.count] = idx;
        self.count += 1;
    }

    fn close(&mut self, idx: usize) {
        self.wins[idx].open = false;
        self.remove_from_order(idx);
    }

    fn hit_test(&self, x: i32, y: i32) -> Option<usize> {
        for k in (0..self.count).rev() {
            let i = self.order[k];
            let w = &self.wins[i];
            if x >= w.x && x < w.x + w.w && y >= w.y && y < w.y + w.h {
                return Some(i);
            }
        }
        None
    }

    fn move_cursor(&mut self, dx: i32, dy: i32) {
        self.mx = (self.mx + dx).clamp(0, self.sw - 1);
        self.my = (self.my + dy).clamp(0, self.sh - 1);
        if let Some((i, ox, oy)) = self.drag {
            self.wins[i].x = self.mx - ox;
            self.wins[i].y = (self.my - oy).max(panel::HEIGHT);
        }
    }

    fn on_left_down(&mut self) {
        let (x, y) = (self.mx, self.my);

        if self.menu {
            self.menu = false;
            if let Some(i) = panel::menu_item_at(x, y) {
                match i {
                    0 => self.open_app(App::Welcome),
                    1 => self.open_app(App::Settings),
                    _ => self.quit = true, // Log out
                }
                return;
            }
            if panel::menu_button_hit(x, y) {
                return; // clicking the button again just closes the menu
            }
        }
        if panel::menu_button_hit(x, y) {
            self.menu = true;
            return;
        }
        if let Some(i) = dock::hit(x, y, self.sw, self.sh) {
            self.open_app(dock::APPS[i]);
            return;
        }
        if let Some(i) = self.hit_test(x, y) {
            self.raise(i);
            let w = self.wins[i];
            let (dx, dy) = (x - (w.x + 18), y - (w.y + TITLE_H / 2));
            if dx * dx + dy * dy <= 81 {
                self.close(i); // the red traffic light
            } else if y < w.y + TITLE_H {
                self.drag = Some((i, x - w.x, y - w.y));
            }
        }
    }

    fn handle(&mut self, ev: Event) {
        match ev {
            Event::MouseMove { dx, dy } => self.move_cursor(dx, dy),
            Event::Button { button: 0, down: true } => self.on_left_down(),
            Event::Button { button: 0, down: false } => self.drag = None,
            Event::Key { key, down: true } => match key {
                // keyboard fallback when there is no mouse
                Key::Up => self.move_cursor(0, -KEY_STEP),
                Key::Down => self.move_cursor(0, KEY_STEP),
                Key::Left => self.move_cursor(-KEY_STEP, 0),
                Key::Right => self.move_cursor(KEY_STEP, 0),
                Key::Enter => {
                    // Enter = click; pressing it again drops a dragged window
                    if self.drag.is_some() {
                        self.drag = None;
                    } else {
                        self.on_left_down();
                    }
                }
                Key::Escape => self.menu = false,
                Key::Char(_) => {}
            },
            _ => {}
        }
        self.dirty = true;
    }

    fn draw(&self, g: &mut Gfx, secs: u64) {
        wallpaper::draw(g);
        for k in 0..self.count {
            draw_window(g, &self.wins[self.order[k]], k + 1 == self.count, secs);
        }

        let mut open = [false; 3];
        for (j, a) in dock::APPS.iter().enumerate() {
            open[j] = self.wins.iter().any(|w| w.open && w.app == *a);
        }
        dock::draw(g, open, dock::hit(self.mx, self.my, self.sw, self.sh));

        let title = if self.count > 0 { self.wins[self.order[self.count - 1]].app.title() } else { "" };
        panel::draw(g, secs, self.menu, title);
        if self.menu {
            panel::draw_menu(g, panel::menu_item_at(self.mx, self.my));
        }
        draw_cursor(g, self.mx, self.my);
    }
}

fn draw_window(g: &mut Gfx, w: &Window, focused: bool, secs: u64) {
    // soft shadow
    for &(i, a) in &[(3i32, 20u32), (8, 14), (14, 9)] {
        g.round_rect(w.x - i, w.y - i + 8, w.w + 2 * i, w.h + 2 * i, 12 + i, 0x000000, 0x000000, a);
    }
    // border, body, title bar
    let border = if focused { 0x7c5cff } else { 0x44445c };
    g.round_rect(w.x, w.y, w.w, w.h, 12, border, border, 255);
    g.round_rect_c(w.x + 1, w.y + TITLE_H, w.w - 2, w.h - TITLE_H - 1, 11, BL | BR, 0x24243a, 0x1d1d30, 255);
    let (t0, t1) = if focused { (0x3d3a66, 0x2c2a4c) } else { (0x30304a, 0x2a2a40) };
    g.round_rect_c(w.x + 1, w.y + 1, w.w - 2, TITLE_H - 1, 11, TL | TR, t0, t1, 255);
    g.rect(w.x + 1, w.y + TITLE_H - 1, w.w - 2, 1, 0xFFFFFF, 16);

    // traffic lights (only the red one does something)
    let ly = w.y + TITLE_H / 2;
    let (red, yellow, green) = if focused { (0xFF5F57, 0xFFBD2E, 0x28C840) } else { (0x555566, 0x555566, 0x555566) };
    g.circle(w.x + 18, ly, 6, red, 255);
    g.circle(w.x + 38, ly, 6, yellow, 255);
    g.circle(w.x + 58, ly, 6, green, 255);

    let title = w.app.title();
    let tw = text_width(title, 1);
    g.text(w.x + (w.w - tw) / 2, w.y + 7, title, if focused { 0xFFFFFF } else { 0x9a9ab0 });

    apps::draw_content(g, w.app, w.x + 1, w.y + TITLE_H, w.w - 2, w.h - TITLE_H - 1, secs);
}

const CURSOR: [&str; 17] = [
    "#           ",
    "##          ",
    "#.#         ",
    "#..#        ",
    "#...#       ",
    "#....#      ",
    "#.....#     ",
    "#......#    ",
    "#.......#   ",
    "#........#  ",
    "#.....#####  ",
    "#..#..#     ",
    "#.# #..#    ",
    "##  #..#    ",
    "#    #..#   ",
    "     #..#   ",
    "      ##    ",
];

fn draw_cursor(g: &mut Gfx, x: i32, y: i32) {
    for (r, row) in CURSOR.iter().enumerate() {
        for (c, ch) in row.bytes().enumerate() {
            if ch != b' ' {
                g.px(x + c as i32 + 1, y + r as i32 + 1, 0x000000, 70); // drop shadow
            }
        }
    }
    for (r, row) in CURSOR.iter().enumerate() {
        for (c, ch) in row.bytes().enumerate() {
            let color = match ch {
                b'#' => 0x000000,
                b'.' => 0xFFFFFF,
                _ => continue,
            };
            g.px(x + c as i32, y + r as i32, color, 255);
        }
    }
}

/// Runs the desktop until the user picks "Log out".
/// `back` is a scratch buffer of at least `width * height` pixels owned by the caller.
pub fn run<H: Host>(host: &mut H, back: &mut [u32]) {
    let (w, h) = host.size();
    let n = w * h;
    if back.len() < n {
        return;
    }
    let font = host.font();
    let mut d = Desktop::new(w as i32, h as i32);
    d.open_app(App::Welcome);

    while !d.quit {
        while let Some(ev) = host.poll_event() {
            d.handle(ev);
        }
        let secs = host.uptime_ms() / 1000;
        if secs != d.last_sec {
            d.last_sec = secs;
            d.dirty = true; // the clock (and the terminal cursor) changed
        }
        if d.dirty {
            {
                let mut g = Gfx::new(&mut back[..n], w, h, font);
                d.draw(&mut g, secs);
            }
            host.present(&back[..n]);
            d.dirty = false;
        }
        host.sleep_ms(8);
    }
}