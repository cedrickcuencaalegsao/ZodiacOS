use crate::package::{self, Package, ALL};
use crate::{kprint, kprintln, vga};
use core::sync::atomic::Ordering::Relaxed;

pub const VERSION: &str = "0.1.0";

#[derive(Clone, Copy, PartialEq)]
enum Filter {
    All,
    Installed,
    Available,
}

pub fn run(args: &[&str]) {
    match args {
        ["list"] | ["ls"] => list(Filter::All),
        ["list", "installed"] => list(Filter::Installed),
        ["list", "available"] => list(Filter::Available),
        ["info", name] => info(name),
        ["install", name] => install(name),
        ["remove", name] => remove(name),
        ["run", name, rest @ ..] => {
            if !launch(name, rest) {
                kprintln!("zpm: package '{}' not found", name);
            }
        }
        _ => usage(),
    }
}

fn usage() {
    kprintln!("zpm {} - Zodiac Package Manager", VERSION);
    kprintln!("  zpm list              list all packages");
    kprintln!("  zpm list installed    list installed packages");
    kprintln!("  zpm list available    list packages you can install");
    kprintln!("  zpm info <name>       show package details");
    kprintln!("  zpm install <name>    install a package");
    kprintln!("  zpm remove <name>     remove a package");
    kprintln!("  zpm run <name> [args] run an installed package");
}

/// Status label and VGA color for a package.
fn status_of(p: &Package) -> (&'static str, u8) {
    if p.builtin {
        ("built-in", 0x0B) // light cyan
    } else if p.installed.load(Relaxed) {
        ("installed", 0x0A) // light green
    } else {
        ("available", 0x0E) // yellow
    }
}

fn list(filter: Filter) {
    vga::set_color(0x07);
    kprintln!("{:<10} {:<8} {:<10} {}", "NAME", "VERSION", "STATUS", "DESCRIPTION");

    let (mut installed, mut available) = (0, 0);

    for p in ALL.iter() {
        let is_installed = p.builtin || p.installed.load(Relaxed);
        if is_installed {
            installed += 1;
        } else {
            available += 1;
        }

        let show = match filter {
            Filter::All => true,
            Filter::Installed => is_installed,
            Filter::Available => !is_installed,
        };
        if !show {
            continue;
        }

        let (label, color) = status_of(p);
        vga::set_color(0x0F);
        kprint!("{:<10} {:<8} ", p.name, p.version);
        vga::set_color(color);
        kprint!("{:<10} ", label);
        vga::set_color(0x07);
        kprintln!("{}", p.description);
    }

    vga::set_color(0x0F);
    kprintln!("\n{} installed, {} available", installed, available);
}

fn info(name: &str) {
    match package::find(name) {
        Some(p) => {
            let (label, color) = status_of(p);
            kprintln!("Name:        {}", p.name);
            kprintln!("Version:     {}", p.version);
            kprintln!("Description: {}", p.description);
            kprint!("Status:      ");
            vga::set_color(color);
            kprintln!("{}", label);
            vga::set_color(0x0F);
        }
        None => kprintln!("zpm: package '{}' not found", name),
    }
}

fn install(name: &str) {
    match package::find(name) {
        None => kprintln!("zpm: package '{}' not found", name),
        Some(p) if p.builtin || p.installed.load(Relaxed) => {
            kprintln!("zpm: '{}' is already installed", p.name)
        }
        Some(p) => {
            p.installed.store(true, Relaxed);
            vga::set_color(0x0A);
            kprintln!("Installed {} {}", p.name, p.version);
            vga::set_color(0x0F);
        }
    }
}

fn remove(name: &str) {
    match package::find(name) {
        None => kprintln!("zpm: package '{}' not found", name),
        Some(p) if p.builtin => kprintln!("zpm: '{}' is built-in and cannot be removed", p.name),
        Some(p) if !p.installed.load(Relaxed) => {
            kprintln!("zpm: '{}' is not installed", p.name)
        }
        Some(p) => {
            p.installed.store(false, Relaxed);
            vga::set_color(0x0E);
            kprintln!("Removed {}", p.name);
            vga::set_color(0x0F);
        }
    }
}

/// Runs an installed package. Returns false only if no such package exists.
pub fn launch(name: &str, args: &[&str]) -> bool {
    match package::find(name) {
        None => false,
        Some(p) => {
            if p.builtin || p.installed.load(Relaxed) {
                (p.entry)(args);
            } else {
                kprintln!("'{}' is not installed. Run: zpm install {}", p.name, p.name);
            }
            true
        }
    }
}