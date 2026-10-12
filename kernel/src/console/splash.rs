use crate::arch::sleep_ms;
use crate::vga::{self, print_at, put_char};

const BAR_LEN: usize = 40;
const LOGO_W: usize = 47; // 8 letters * 5 cols + 7 gaps
const CONTENT_H: usize = 13;
const REVEAL_MS: u32 = 70; // per letter
const STAR_COUNT: usize = 48;

const SPINNER: [u8; 4] = [b'|', b'/', b'-', b'\\'];
const ROW_COLORS: [u8; 5] = [0x0B, 0x0B, 0x03, 0x03, 0x09]; // cyan -> blue gradient
const TWINKLE: [(u8, u8); 4] = [(0xFA, 0x08), (0xF9, 0x07), (b'.', 0x0F), (b'*', 0x0F)];

const STATUS: [&[u8]; 5] = [
    b"Initializing memory",
    b"Probing PCI bus",
    b"Loading drivers",
    b"Starting scheduler",
    b"Aligning the stars",
];

// CP437 box-drawing and block characters
const BLOCK: u8 = 0xDB;
const SHADE: u8 = 0xB0;
const TL: u8 = 0xC9;
const TR: u8 = 0xBB;
const BL: u8 = 0xC8;
const BR: u8 = 0xBC;
const HZ: u8 = 0xCD;
const VT: u8 = 0xBA;
const CHECK: u8 = 0xFB;

fn glyph(c: u8) -> [u8; 5] {
    match c {
        b'Z' => [0b11111, 0b00010, 0b00100, 0b01000, 0b11111],
        b'O' => [0b01110, 0b10001, 0b10001, 0b10001, 0b01110],
        b'D' => [0b11110, 0b10001, 0b10001, 0b10001, 0b11110],
        b'I' => [0b11111, 0b00100, 0b00100, 0b00100, 0b11111],
        b'A' => [0b01110, 0b10001, 0b11111, 0b10001, 0b10001],
        b'C' => [0b01111, 0b10000, 0b10000, 0b10000, 0b01111],
        b'S' => [0b01111, 0b10000, 0b01110, 0b00001, 0b11110],
        _ => [0; 5],
    }
}

fn draw_glyph(row: usize, col: usize, g: [u8; 5]) {
    for (y, bits) in g.iter().enumerate() {
        for x in 0..5 {
            if bits >> (4 - x) & 1 == 1 {
                put_char(row + y, col + x, BLOCK, ROW_COLORS[y]);
            }
        }
    }
}

fn center(width: usize, len: usize) -> usize {
    width.saturating_sub(len) / 2
}

/// Tiny LCG so the starfield is the same every boot without needing an RNG.
struct Rng(u32);
impl Rng {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        self.0 >> 8
    }
}

fn make_stars(
    rng: &mut Rng,
    w: usize,
    h: usize,
    top: usize,
    left: usize,
    right: usize,
) -> ([(usize, usize); STAR_COUNT], usize) {
    let mut stars = [(0, 0); STAR_COUNT];
    let mut n = 0;
    let mut attempts = 0;
    while n < STAR_COUNT && attempts < 1000 {
        attempts += 1;
        let r = rng.next() as usize % h;
        let c = rng.next() as usize % w;
        // keep the middle clear for the logo and bar
        let in_content = r + 1 >= top && r <= top + CONTENT_H && c + 6 >= left && c <= right + 6;
        if !in_content {
            stars[n] = (r, c);
            n += 1;
            let (ch, color) = TWINKLE[rng.next() as usize % TWINKLE.len()];
            put_char(r, c, ch, color);
        }
    }
    (stars, n)
}

fn draw_status(row: usize, w: usize, msg: &[u8]) {
    // clear the old message, then centre the new one
    for c in 0..BAR_LEN + 2 {
        put_char(row, center(w, BAR_LEN + 2) + c, b' ', 0x07);
    }
    print_at(row, center(w, msg.len()), msg, 0x07);
}

fn draw_percent(row: usize, col: usize, p: usize) {
    put_char(row, col, if p >= 100 { b'1' } else { b' ' }, 0x0F);
    put_char(row, col + 1, if p >= 10 { b'0' + (p / 10 % 10) as u8 } else { b' ' }, 0x0F);
    put_char(row, col + 2, b'0' + (p % 10) as u8, 0x0F);
    put_char(row, col + 3, b'%', 0x07);
}

pub fn run(total_ms: u32) {
    let (w, h) = (vga::width(), vga::height());
    let top = h.saturating_sub(CONTENT_H) / 2;
    let logo_col = center(w, LOGO_W);
    let bar_col = center(w, BAR_LEN + 2);

    let tagline_row = top + 6;
    let frame_row = top + 8;
    let bar_row = frame_row + 1;
    let status_row = top + 12;

    // Starfield
    let mut rng = Rng(0x5EED_2024);
    let (stars, star_n) = make_stars(&mut rng, w, h, top, logo_col, logo_col + LOGO_W);

    // Logo, revealed one letter at a time
    for (i, &c) in b"ZODIACOS".iter().enumerate() {
        draw_glyph(top, logo_col + i * 6, glyph(c));
        sleep_ms(REVEAL_MS as u32);
    }

    let tag = b"a tiny kernel, written in Rust";
    print_at(tagline_row, center(w, tag.len()), tag, 0x08);

    // Progress bar frame
    put_char(frame_row, bar_col, TL, 0x03);
    put_char(frame_row, bar_col + BAR_LEN + 1, TR, 0x03);
    put_char(frame_row + 2, bar_col, BL, 0x03);
    put_char(frame_row + 2, bar_col + BAR_LEN + 1, BR, 0x03);
    put_char(bar_row, bar_col, VT, 0x03);
    put_char(bar_row, bar_col + BAR_LEN + 1, VT, 0x03);
    for i in 0..BAR_LEN {
        put_char(frame_row, bar_col + 1 + i, HZ, 0x03);
        put_char(frame_row + 2, bar_col + 1 + i, HZ, 0x03);
        put_char(bar_row, bar_col + 1 + i, SHADE, 0x08);
    }

    let spin_col = bar_col.saturating_sub(3);
    let pct_col = bar_col + BAR_LEN + 3;
    let half_ms = total_ms.saturating_sub(REVEAL_MS * 8) / (BAR_LEN as u32 * 2);

    let mut frame = 0usize;
    let mut last_msg = usize::MAX;

    for i in 0..BAR_LEN {
        let msg = i * STATUS.len() / BAR_LEN;
        if msg != last_msg {
            draw_status(status_row, w, STATUS[msg]);
            last_msg = msg;
        }

        for _ in 0..2 {
            put_char(bar_row, spin_col, SPINNER[frame % 4], 0x0E);
            frame += 1;

            // twinkle a couple of stars
            if star_n > 0 {
                for _ in 0..2 {
                    let (r, c) = stars[rng.next() as usize % star_n];
                    let (ch, color) = TWINKLE[rng.next() as usize % TWINKLE.len()];
                    put_char(r, c, ch, color);
                }
            }
            sleep_ms(half_ms as u32);
        }

        // fill: bright head, solid trail
        put_char(bar_row, bar_col + 1 + i, BLOCK, 0x0F);
        if i > 0 {
            put_char(bar_row, bar_col + i, BLOCK, 0x0B);
        }
        draw_percent(bar_row, pct_col, (i + 1) * 100 / BAR_LEN);
    }

    // Done: settle the bar, swap spinner for a check mark
    put_char(bar_row, bar_col + BAR_LEN, BLOCK, 0x0B);
    put_char(bar_row, spin_col, CHECK, 0x0A);
    draw_status(status_row, w, b"Welcome to ZodiacOS");
    sleep_ms(500);
}