//! `zpm` command table for the shell (`terminal.rs`).
//!
//! This file only maps subcommands to `zpm_core::commands`; the logic lives
//! in the zpm_core crate.

use core::fmt::Write;

use zpm_core::{commands, Registry};

/// `reg`     = installed packages (changed by install / remove / upgrade).
/// `catalog` = every package that can be installed, at its latest version.
pub fn run<const N: usize, const M: usize, W: Write>(
    args: &str,
    reg: &mut Registry<N>,
    catalog: &Registry<M>,
    out: &mut W,
) {
    let mut it = args.split_whitespace();
    match it.next() {
        Some("list" | "ls") => commands::list(reg, catalog, it.next(), out),
        Some("search") => commands::search(reg, catalog, it.next(), out),
        Some("info") => match it.next() {
            Some(name) => commands::info(reg, catalog, name, out),
            None => {
                let _ = writeln!(out, "usage: zpm info <package>");
            }
        },
        Some("install") => match it.next() {
            Some(name) => commands::install(reg, catalog, name, out),
            None => {
                let _ = writeln!(out, "usage: zpm install <package>");
            }
        },
        Some("remove") => match it.next() {
            Some(name) => commands::remove(reg, name, out),
            None => {
                let _ = writeln!(out, "usage: zpm remove <package>");
            }
        },
        Some("update" | "upgrade") => commands::upgrade(reg, catalog, it.next(), out),
        Some("download") => {
            let _ = writeln!(out, "zpm: download needs networking, which ZodiacOS doesn't have yet.");
            let _ = writeln!(out, "     Run it on the host: zpm --root <image-root> download");
        }
        _ => usage(out),
    }
}

fn usage<W: Write>(out: &mut W) {
    let _ = writeln!(out, "zpm - Zodiac Package Manager");
    let _ = writeln!(out, "  zpm list [filter]      installed packages");
    let _ = writeln!(out, "  zpm search [filter]    packages you can install");
    let _ = writeln!(out, "  zpm info <package>     package details");
    let _ = writeln!(out, "  zpm install <package>  install a package");
    let _ = writeln!(out, "  zpm remove <package>   remove a package");
    let _ = writeln!(out, "  zpm upgrade [package]  upgrade one or all packages");
}