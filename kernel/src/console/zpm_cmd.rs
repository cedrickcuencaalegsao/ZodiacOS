use core::fmt::Write;
use zpm_core::{Registry, RegistryError, Version};

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

fn key(v: Version) -> (u16, u16, u16) {
    (v.major, v.minor, v.patch)
}

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
        // ---------------------------------------------------------- list
        Some("list" | "ls") => {
            let filter = it.next();

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
                    Some(q) if key(q.version) > key(p.version) => {
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

        // -------------------------------------------------------- search
        // Shows everything in the catalog and whether it is installed.
        Some("search") => {
            let filter = it.next();

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

        // ---------------------------------------------------------- info
        Some("info") => match it.next() {
            None => {
                let _ = writeln!(out, "usage: zpm info <package>");
            }
            Some(name) => match catalog.get(name).or_else(|| reg.get(name)) {
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
            },
        },

        // ------------------------------------------------------- install
        Some("install") => match it.next() {
            None => {
                let _ = writeln!(out, "usage: zpm install <package>");
            }
            Some(name) => match catalog.get(name) {
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
            },
        },

        // -------------------------------------------------------- remove
        Some("remove") => match it.next() {
            None => {
                let _ = writeln!(out, "usage: zpm remove <package>");
            }
            Some(name) => match reg.remove(name) {
                Ok(()) => {
                    let _ = writeln!(out, "Removed {}", name);
                }
                Err(_) => {
                    let _ = writeln!(out, "zpm: '{}' is not installed", name);
                }
            },
        },

        // ------------------------------------------------ update/upgrade
        // `zpm upgrade` upgrades everything, `zpm upgrade <package>` just one.
        Some("update" | "upgrade") => {
            let only = it.next();
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
                if key(q.version) <= key(old) {
                    continue;
                }
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

        // ---------------------------------------------------- host only
        Some("download") => {
            let _ = writeln!(out, "zpm: download needs networking, which ZodiacOS doesn't have yet.");
            let _ = writeln!(out, "     Run it on the host: zpm --root <image-root> download");
        }

        _ => {
            let _ = writeln!(out, "zpm - Zodiac Package Manager");
            let _ = writeln!(out, "  zpm list [filter]      installed packages");
            let _ = writeln!(out, "  zpm search [filter]    packages you can install");
            let _ = writeln!(out, "  zpm info <package>     package details");
            let _ = writeln!(out, "  zpm install <package>  install a package");
            let _ = writeln!(out, "  zpm remove <package>   remove a package");
            let _ = writeln!(out, "  zpm upgrade [package]  upgrade one or all packages");
        }
    }
}