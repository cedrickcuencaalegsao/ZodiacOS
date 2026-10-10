use super::Package;
use crate::{kprint, kprintln, vga};
use core::arch::x86_64::__cpuid;
use core::sync::atomic::AtomicBool;

pub static PACKAGE: Package = Package {
    name: "sysinfo",
    version: "0.1.0",
    description: "Shows basic system information",
    builtin: true,
    installed: AtomicBool::new(true),
    entry: main,
};

// Orion. Every logo line must be exactly LOGO_WIDTH characters wide.
//   '*' bright star   '.' faint star   / \ - |  lines between the stars
const LOGO_WIDTH: usize = 20;
const LOGO: [&str; 10] = [
    r"  *       .       * ",
    r"   \             /  ",
    r"    \           /   ",
    r"     \    .    /    ",
    r"      \       /     ",
    r"       *--*--*      ",
    r"      /   |   \     ",
    r"     /    *    \    ",
    r"    /     .     \   ",
    r"   *             *  ",
];

// Compile-time check: the build fails if any logo line has the wrong width.
const _: () = {
    let mut i = 0;
    while i < LOGO.len() {
        assert!(LOGO[i].len() == LOGO_WIDTH, "logo line has the wrong width");
        i += 1;
    }
};

// VGA colors
const STAR_COLOR: u8 = 0x0E; // yellow
const DIM_COLOR: u8 = 0x08; // dark gray
const LINE_COLOR: u8 = 0x03; // cyan
const KEY_COLOR: u8 = 0x0B; // light cyan
const TEXT_COLOR: u8 = 0x0F; // white

// Code page 437 bytes from the default VGA font
const BRIGHT_STAR: u8 = 0x04; // ♦
const FAINT_STAR: u8 = 0xFA; // ·

fn print_logo_line(line: &str) {
    for ch in line.bytes() {
        let (shown, color) = match ch {
            b'*' => (BRIGHT_STAR, STAR_COLOR),
            b'.' => (FAINT_STAR, DIM_COLOR),
            b' ' => (b' ', TEXT_COLOR),
            _ => (ch, LINE_COLOR),
        };
        vga::set_color(color);
        vga::put_byte(shown);
    }
}

// One row of the right-hand (info) column.
enum InfoLine<'a> {
    Empty,
    Title,
    Rule,
    Field(&'static str, &'a str),
    Palette,
}

fn info_line<'a>(row: usize, vendor: &'a str) -> InfoLine<'a> {
    match row {
        2 => InfoLine::Title,
        3 => InfoLine::Rule,
        4 => InfoLine::Field("OS", "ZodiacOS 0.1.0"),
        5 => InfoLine::Field("Arch", "x86_64 (long mode)"),
        6 => InfoLine::Field("CPU", vendor),
        7 => InfoLine::Field("Display", "VGA text 80x25"),
        9 => InfoLine::Palette,
        _ => InfoLine::Empty,
    }
}

fn print_info(line: &InfoLine) {
    match line {
        InfoLine::Empty => {}
        InfoLine::Title => {
            vga::set_color(STAR_COLOR);
            kprint!("root@zodiac");
        }
        InfoLine::Rule => {
            vga::set_color(DIM_COLOR);
            kprint!("-----------");
        }
        InfoLine::Field(key, value) => {
            vga::set_color(KEY_COLOR);
            kprint!("{}", key);
            vga::set_color(TEXT_COLOR);
            kprint!(": {}", value);
        }
        InfoLine::Palette => {
            // 0xDB is the full-block character
            for c in [0x0C, 0x0E, 0x0A, 0x0B, 0x09, 0x0D, 0x0F, 0x08] {
                vga::set_color(c);
                vga::put_byte(0xDB);
                vga::put_byte(0xDB);
            }
        }
    }
}

fn main(_args: &[&str]) {
    #[allow(unused_unsafe)]
    let r = unsafe { __cpuid(0) };

    let mut vendor_bytes = [0u8; 12];
    vendor_bytes[0..4].copy_from_slice(&r.ebx.to_le_bytes());
    vendor_bytes[4..8].copy_from_slice(&r.edx.to_le_bytes());
    vendor_bytes[8..12].copy_from_slice(&r.ecx.to_le_bytes());
    let vendor = core::str::from_utf8(&vendor_bytes).unwrap_or("unknown");

    kprintln!();
    for (i, logo_line) in LOGO.iter().enumerate() {
        kprint!("  ");
        print_logo_line(logo_line);
        kprint!("  ");
        print_info(&info_line(i, vendor));
        vga::set_color(TEXT_COLOR);
        kprintln!();
    }
    kprintln!();
}