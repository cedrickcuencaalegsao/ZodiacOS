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

const CLOSED: Window = Window {
    x: 0,
    y: 0,
    w: 0,
    h: 0,
    app: App::Welcome,
    open: false,
};

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
    scene_dirty: bool,  // windows/dock/panel must be redrawn
    cursor_dirty: bool, // only the pointer moved
    last_key: u32,      // hover state of everything that reacts to the pointer
    shown: (i32, i32),  // where the cursor is currently painted
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
            scene_dirty: true,
            cursor_dirty: true,
            last_key: 0,
            shown: (sw / 2, sh / 2),
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
            self.wins[i] = Window {
                x,
                y,
                w,
                h,
                app,
                open: true,
            };
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
            Event::MouseMove { dx, dy } => {
                if dx == 0 && dy == 0 {
                    return; // nothing changed, no redraw
                }
                self.move_cursor(dx, dy);
                if self.drag.is_some() {
                    self.scene_dirty = true; // a window is moving
                } else {
                    self.cursor_dirty = true; // just the pointer
                }
                return;
            }
            Event::Button {
                button: 0,
                down: true,
            } => self.on_left_down(),
            Event::Button {
                button: 0,
                down: false,
            } => self.drag = None,
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
            _ => return,
        }
        self.scene_dirty = true;
    }

    /// Everything that changes appearance when the pointer hovers it.
    fn hover_key(&self) -> u32 {
        let dock_h = dock::hit(self.mx, self.my, self.sw, self.sh).map_or(0, |i| i as u32 + 1);
        let menu_h = if self.menu {
            panel::menu_item_at(self.mx, self.my).map_or(0, |i| i as u32 + 1)
        } else {
            0
        };
        let mut close = 0u32;
        for i in 0..MAX_WIN {
            let w = &self.wins[i];
            let (dx, dy) = (self.mx - (w.x + 18), self.my - (w.y + TITLE_H / 2));
            if w.open && dx * dx + dy * dy <= 81 {
                close |= 1u32 << i;
            }
        }
        dock_h | (menu_h << 4) | (close << 8)
    }

    /// Scene plus pointer (used when there is no room for the partial-update buffers).
    fn draw(&self, g: &mut Gfx, secs: u64, with_wallpaper: bool) {
        self.draw_scene(g, secs, with_wallpaper);
        draw_cursor(g, self.mx, self.my);
    }

    /// Everything except the pointer. `with_wallpaper`: false when a cached wallpaper was already copied in.
    fn draw_scene(&self, g: &mut Gfx, secs: u64, with_wallpaper: bool) {
        if with_wallpaper {
            wallpaper::draw(g);
        }
        for k in 0..self.count {
            let w = &self.wins[self.order[k]];
            let dx = self.mx - (w.x + 18);
            let dy = self.my - (w.y + TITLE_H / 2);
            let close_hover = dx * dx + dy * dy <= 81;
            draw_window(g, w, k + 1 == self.count, close_hover, secs);
        }

        let mut open = [false; 3];
        for (j, a) in dock::APPS.iter().enumerate() {
            open[j] = self.wins.iter().any(|w| w.open && w.app == *a);
        }
        dock::draw(g, open, dock::hit(self.mx, self.my, self.sw, self.sh));

        let title = if self.count > 0 {
            self.wins[self.order[self.count - 1]].app.title()
        } else {
            ""
        };
        panel::draw(g, secs, self.menu, title);
        if self.menu {
            panel::draw_menu(g, panel::menu_item_at(self.mx, self.my));
        }
    }
}

fn draw_window(g: &mut Gfx, w: &Window, focused: bool, close_hover: bool, secs: u64) {
    // layered soft shadow (deeper when focused)
    let shadow: &[(i32, u32)] = if focused {
        &[(3, 26), (8, 20), (14, 14), (22, 8)]
    } else {
        &[(3, 16), (8, 11), (14, 7)]
    };
    for &(i, a) in shadow {
        g.round_rect(
            w.x - i,
            w.y - i + 10,
            w.w + 2 * i,
            w.h + 2 * i,
            12 + i,
            0x000000,
            0x000000,
            a,
        );
    }
    // faint violet glow around the focused window
    if focused {
        g.round_rect(
            w.x - 2,
            w.y - 2,
            w.w + 4,
            w.h + 4,
            14,
            0x7c5cff,
            0x7c5cff,
            38,
        );
    }
    // border, body, title bar
    let border = if focused { 0x8a6bff } else { 0x44445c };
    g.round_rect(w.x, w.y, w.w, w.h, 12, border, border, 255);
    g.round_rect_c(
        w.x + 1,
        w.y + TITLE_H,
        w.w - 2,
        w.h - TITLE_H - 1,
        11,
        BL | BR,
        0x26263e,
        0x1a1a2c,
        255,
    );
    let (t0, t1) = if focused {
        (0x46427a, 0x2c2a4c)
    } else {
        (0x30304a, 0x2a2a40)
    };
    g.round_rect_c(
        w.x + 1,
        w.y + 1,
        w.w - 2,
        TITLE_H - 1,
        11,
        TL | TR,
        t0,
        t1,
        255,
    );
    // top highlight + separator
    g.rect(w.x + 8, w.y + 1, w.w - 16, 1, 0xFFFFFF, 40);
    g.rect(w.x + 1, w.y + TITLE_H - 1, w.w - 2, 1, 0x000000, 70);
    g.rect(w.x + 1, w.y + TITLE_H, w.w - 2, 1, 0xFFFFFF, 14);

    // traffic lights (only the red one does something)
    let ly = w.y + TITLE_H / 2;
    let (red, yellow, green) = if focused {
        (0xFF5F57, 0xFFBD2E, 0x28C840)
    } else {
        (0x555566, 0x555566, 0x555566)
    };
    g.circle(w.x + 18, ly, 7, 0x000000, 60); // ring behind each light
    g.circle(w.x + 38, ly, 7, 0x000000, 60);
    g.circle(w.x + 58, ly, 7, 0x000000, 60);
    g.circle(
        w.x + 18,
        ly,
        6,
        if close_hover { 0xFF8A84 } else { red },
        255,
    );
    g.circle(w.x + 38, ly, 6, yellow, 255);
    g.circle(w.x + 58, ly, 6, green, 255);
    g.circle(w.x + 18, ly - 2, 3, 0xFFFFFF, 40); // gloss
    if close_hover {
        let c = w.x + 18;
        g.line(c - 2, ly - 2, c + 2, ly + 2, 0x5a0a06, 230);
        g.line(c - 2, ly + 2, c + 2, ly - 2, 0x5a0a06, 230);
    }

    let title = w.app.title();
    let tw = text_width(title, 1);
    let tx = w.x + (w.w - tw) / 2;
    if focused {
        g.text(tx, w.y + 8, title, 0x000000); // subtle text shadow
    }
    g.text(
        tx,
        w.y + 7,
        title,
        if focused { 0xFFFFFF } else { 0x9a9ab0 },
    );

    apps::draw_content(
        g,
        w.app,
        w.x + 1,
        w.y + TITLE_H,
        w.w - 2,
        w.h - TITLE_H - 1,
        secs,
    );
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

const CUR_W: i32 = 16;
const CUR_H: i32 = 20;

/// Runs the desktop until the user picks "Log out".
///
/// `back` is a scratch buffer owned by the caller:
/// - `3 * width * height` pixels: best. Pointer-only movement repaints just the cursor area
///   (`Host::present_rect`), which is what stops the flicker.
/// - `2 * width * height`: wallpaper is cached, but every move presents the whole frame.
/// - `width * height`: minimal; everything is redrawn each time.
pub fn run<H: Host>(host: &mut H, back: &mut [u32]) {
    let (w, h) = host.size();
    let n = w * h;
    if back.len() < n {
        return;
    }
    let font = host.font();
    let (frame, rest) = back.split_at_mut(n);
    let (scene, wall): (&mut [u32], &mut [u32]) = if rest.len() >= 2 * n {
        let (a, b) = rest.split_at_mut(n);
        (a, &mut b[..n])
    } else if rest.len() >= n {
        (&mut [], &mut rest[..n])
    } else {
        (&mut [], &mut [])
    };
    let fast = scene.len() == n;
    let cached = wall.len() == n;
    if cached {
        let mut g = Gfx::new(&mut wall[..], w, h, font);
        wallpaper::draw(&mut g);
    }

    let mut d = Desktop::new(w as i32, h as i32);
    d.open_app(App::Welcome);

    while !d.quit {
        while let Some(ev) = host.poll_event() {
            d.handle(ev);
        }
        let secs = host.uptime_ms() / 1000;
        if secs != d.last_sec {
            d.last_sec = secs;
            d.scene_dirty = true; // the clock (and the terminal cursor) changed
        }
        let key = d.hover_key();
        if key != d.last_key {
            d.last_key = key;
            d.scene_dirty = true; // a hover highlight appeared or went away
        }

        if fast && !d.scene_dirty && d.cursor_dirty {
            // pointer only: patch the old and new cursor areas, push just that rectangle
            let (ox, oy) = d.shown;
            let (nx, ny) = (d.mx, d.my);
            let x0 = ox.min(nx).max(0);
            let y0 = oy.min(ny).max(0);
            let x1 = (ox.max(nx) + CUR_W).min(w as i32);
            let y1 = (oy.max(ny) + CUR_H).min(h as i32);
            for row in y0..y1 {
                let a = row as usize * w + x0 as usize;
                let b = row as usize * w + x1 as usize;
                frame[a..b].copy_from_slice(&scene[a..b]);
            }
            {
                let mut g = Gfx::new(&mut frame[..], w, h, font);
                draw_cursor(&mut g, nx, ny);
            }
            host.present_rect(
                &frame[..],
                x0 as usize,
                y0 as usize,
                (x1 - x0) as usize,
                (y1 - y0) as usize,
            );
            d.shown = (nx, ny);
            d.cursor_dirty = false;
        } else if d.scene_dirty || d.cursor_dirty {
            if fast {
                scene.copy_from_slice(&wall[..]);
                {
                    let mut g = Gfx::new(&mut scene[..], w, h, font);
                    d.draw_scene(&mut g, secs, false);
                }
                frame.copy_from_slice(&scene[..]);
                let mut g = Gfx::new(&mut frame[..], w, h, font);
                draw_cursor(&mut g, d.mx, d.my);
            } else {
                if cached {
                    frame.copy_from_slice(&wall[..]);
                }
                let mut g = Gfx::new(&mut frame[..], w, h, font);
                d.draw(&mut g, secs, !cached);
            }
            host.present(&frame[..]);
            d.shown = (d.mx, d.my);
            d.scene_dirty = false;
            d.cursor_dirty = false;
        }
        host.sleep_ms(4);
    }
}
