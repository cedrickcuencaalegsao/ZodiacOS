//! Shell-level package commands (`no_std`, no heap).
//!
//! Every function writes its output to a `core::fmt::Write`, so the kernel can
//! pass its console writer and a host program can pass a `String`.

use core::fmt::Write;

use crate::{Registry, RegistryError, Version};

/// Tiny fixed buffer so versions can be padded like normal strings (no alloc).
struct Buf {
    b: [u8; 24],
    n: usize,
}

impl Buf {
    fn new() -> Self {
        Buf { b: [0; 24], n: 0 }
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.b[..self.n]).unwrap_or("?")
    }
}

impl Write for Buf {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let bytes = s.as_bytes();
        if self.n + bytes.len() > self.b.len() {
            return Err(core::fmt::Error);
        }
        self.b[self.n..self.n + bytes.len()].copy_from_slice(bytes);
        self.n += bytes.len();
        Ok(())
    }
}

fn fmt_ver(v: Version) -> Buf {
    let mut b = Buf::new();
    let _ = write!(b, "{}.{}.{}", v.major, v.minor, v.patch);
    b
}

/// `zpm list [filter]`: installed packages and whether an update exists.
pub fn list<const N: usize, const M: usize, W: Write>(
    reg: &Registry<N>,
    catalog: &Registry<M>,
    filter: Option<&str>,
    out: &mut W,
) {
    let _ = writeln!(out, "{:<28} {:<10} {:<14} {}", "NAME", "INSTALLED", "UPDATE", "KIND");
    let _ = writeln!(out, "{:-<28} {:-<10} {:-<14} {:-<10}", "", "", "", "");

    let (mut n, mut updates) = (0, 0);
    for p in reg.iter() {
        if let Some(f) = filter {
            if !p.id.as_str().contains(f) {
                continue;
            }
        }
        let installed = fmt_ver(p.version);

        let mut update = Buf::new();
        match catalog.get(p.id.as_str()) {
            Some(q) if q.version > p.version => {
                let nv = fmt_ver(q.version);
                let _ = write!(update, "-> {}", nv.as_str());
                updates += 1;
            }
            _ => {
                let _ = write!(update, "-");
            }
        }

        let _ = writeln!(
            out,
            "{:<28} {:<10} {:<14} {:?}",
            p.id.as_str(),
            installed.as_str(),
            update.as_str(),
            p.kind
        );
        n += 1;
    }

    if n == 0 {
        let _ = writeln!(out, "(no packages installed)");
    } else {
        let _ = writeln!(out, "\n{} installed, {} update(s) available", n, updates);
    }
}

/// `zpm search [filter]`: everything in the catalog and whether it is installed.
pub fn search<const N: usize, const M: usize, W: Write>(
    reg: &Registry<N>,
    catalog: &Registry<M>,
    filter: Option<&str>,
    out: &mut W,
) {
    let _ = writeln!(out, "{:<28} {:<10} {:<10} {}", "NAME", "VERSION", "STATUS", "KIND");
    let _ = writeln!(out, "{:-<28} {:-<10} {:-<10} {:-<10}", "", "", "", "");

    let mut n = 0;
    for q in catalog.iter() {
        if let Some(f) = filter {
            if !q.id.as_str().contains(f) {
                continue;
            }
        }
        let status = if reg.get(q.id.as_str()).is_some() { "installed" } else { "available" };
        let ver = fmt_ver(q.version);
        let _ = writeln!(out, "{:<28} {:<10} {:<10} {:?}", q.id.as_str(), ver.as_str(), status, q.kind);
        n += 1;
    }
    if n == 0 {
        let _ = writeln!(out, "(nothing found)");
    }
}

/// `zpm info <package>`: details of a catalog (or installed) package.
pub fn info<const N: usize, const M: usize, W: Write>(
    reg: &Registry<N>,
    catalog: &Registry<M>,
    name: &str,
    out: &mut W,
) {
    match catalog.get(name).or_else(|| reg.get(name)) {
        Some(p) => {
            let status = if reg.get(name).is_some() { "installed" } else { "available" };
            let _ = writeln!(out, "name:    {}", p.id.as_str());
            let _ = writeln!(out, "version: {}", fmt_ver(p.version).as_str());
            let _ = writeln!(out, "kind:    {:?}", p.kind);
            let _ = writeln!(out, "status:  {}", status);
        }
        None => {
            let _ = writeln!(out, "zpm: package '{}' not found", name);
        }
    }
}

/// `zpm install <package>`: records a catalog package as installed.
pub fn install<const N: usize, const M: usize, W: Write>(
    reg: &mut Registry<N>,
    catalog: &Registry<M>,
    name: &str,
    out: &mut W,
) {
    match catalog.get(name) {
        None => {
            let _ = writeln!(out, "zpm: package '{}' not found", name);
        }
        Some(q) => match reg.install(*q) {
            Ok(()) => {
                let _ = writeln!(out, "Installed {} {}", name, fmt_ver(q.version).as_str());
            }
            Err(RegistryError::AlreadyInstalled) => {
                let _ = writeln!(out, "zpm: '{}' is already installed", name);
            }
            Err(RegistryError::Full) => {
                let _ = writeln!(out, "zpm: no free slots in the package registry");
            }
            Err(RegistryError::NotFound) => {
                let _ = writeln!(out, "zpm: package '{}' not found", name);
            }
        },
    }
}

/// `zpm remove <package>`.
pub fn remove<const N: usize, W: Write>(reg: &mut Registry<N>, name: &str, out: &mut W) {
    match reg.remove(name) {
        Ok(()) => {
            let _ = writeln!(out, "Removed {}", name);
        }
        Err(_) => {
            let _ = writeln!(out, "zpm: '{}' is not installed", name);
        }
    }
}

/// `zpm upgrade [package]`: upgrades everything, or just `only`.
pub fn upgrade<const N: usize, const M: usize, W: Write>(
    reg: &mut Registry<N>,
    catalog: &Registry<M>,
    only: Option<&str>,
    out: &mut W,
) {
    let mut done = 0;

    for q in catalog.iter() {
        if let Some(name) = only {
            if q.id.as_str() != name {
                continue;
            }
        }
        let old = match reg.get(q.id.as_str()) {
            Some(p) => p.version,
            None => continue,
        };
        if q.version <= old {
            continue;
        }
        // Removing first frees a slot, so the install below cannot hit `Full`.
        let _ = reg.remove(q.id.as_str());
        if reg.install(*q).is_ok() {
            let _ = writeln!(
                out,
                "Upgraded {} {} -> {}",
                q.id.as_str(),
                fmt_ver(old).as_str(),
                fmt_ver(q.version).as_str()
            );
            done += 1;
        }
    }

    if done == 0 {
        let _ = writeln!(out, "Everything is up to date");
    }
}