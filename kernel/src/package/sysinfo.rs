use super::Package;
use crate::kprintln;
use core::arch::x86_64::__cpuid;
use core::fmt;
use core::sync::atomic::AtomicBool;

pub static PACKAGE: Package = Package {
    name: "sysinfo",
    version: "0.1.0",
    description: "Shows basic system information",
    builtin: true,
    installed: AtomicBool::new(true),
    entry: main,
};

// Every logo line must be exactly LOGO_WIDTH characters wide.
const LOGO_WIDTH: usize = 20;
const LOGO: [&str; 9] = [
    r"        /\          ",
    r"       /  \         ",
    r"      / ** \        ",
    r"     /  **  \       ",
    r"    /--------\      ",
    r"    \   **   /      ",
    r"     \  **  /       ",
    r"      \    /        ",
    r"       \  /         ",
];

// Compile-time check: the build fails if any logo line has the wrong width.
const _: () = {
    let mut i = 0;
    while i < LOGO.len() {
        assert!(LOGO[i].len() == LOGO_WIDTH, "logo line has the wrong width");
        i += 1;
    }
};

// One row of the right-hand (info) column.
enum InfoLine<'a> {
    Empty,
    Title,
    Rule,
    Field(&'static str, &'a str),
}

impl fmt::Display for InfoLine<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InfoLine::Empty => Ok(()),
            InfoLine::Title => write!(f, "root@zodiac"),
            InfoLine::Rule => write!(f, "-----------"),
            InfoLine::Field(key, value) => write!(f, "{}: {}", key, value),
        }
    }
}

fn info_line<'a>(row: usize, vendor: &'a str) -> InfoLine<'a> {
    match row {
        1 => InfoLine::Title,
        2 => InfoLine::Rule,
        3 => InfoLine::Field("OS", "ZodiacOS 0.1.0"),
        4 => InfoLine::Field("Arch", "x86_64 (long mode)"),
        5 => InfoLine::Field("CPU", vendor),
        6 => InfoLine::Field("Display", "VGA text 80x25"),
        _ => InfoLine::Empty,
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

    kprintln!("");
    for (i, logo_line) in LOGO.iter().enumerate() {
        kprintln!("  {}  {}", logo_line, info_line(i, vendor));
    }
    kprintln!("");
}