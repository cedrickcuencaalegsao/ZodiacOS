use crate::drivers::storage;
use crate::filesystem::file::{self, O_APPEND, O_CREAT, O_RDONLY, O_TRUNC, O_WRONLY};
use crate::filesystem::path::{resolve_against, split_parent};
use crate::filesystem::{self, FileType, Level, Stat, level_of, mount};
use crate::{kprint, kprintln, time, vga};
use alloc::{format, string::String, vec::Vec};
use core::cmp::Ordering;
use errors::Result;

#[derive(Default, Clone, Copy)]
struct Opts {
    long: bool,
    human: bool,
    reverse: bool,
    by_time: bool,
    by_size: bool,
}

struct Entry {
    name: String,
    level: Level,
    st: Option<Stat>,
}

pub fn run(cmd: &str, args: &[&str]) {
    match cmd {
        "ls" => ls(args),
        "cd" => cd(args),
        "pwd" => pwd(),
        "cat" => cat(args),
        "newcluster" => make(args, cmd, Level::Cluster),
        "newpkg" => make(args, cmd, Level::Package),
        "newfldr" => make(args, cmd, Level::Folder),
        "newfl" => new_item(args),
        "rm" => each(args, cmd, remove),
        "write" => write(args),
        "stat" => stat(args),
        "mount" => mounts(),
        "umount" => each(args, cmd, |p| mount::umount(&abs(p)?)),
        "df" => df(),
        "lsblk" | "drives" => lsblk(),
        _ => {}
    }
}

// ---- helpers ----------------------------------------------------------------

fn abs(path: &str) -> Result<String> {
    resolve_against(&file::getcwd(), path)
}

fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{}{}", dir, name)
    } else {
        format!("{}/{}", dir, name)
    }
}

fn each(args: &[&str], cmd: &str, op: impl Fn(&str) -> Result<()>) {
    if args.is_empty() {
        kprintln!("usage: {} <path...>", cmd);
        return;
    }
    for path in args {
        if let Err(e) = op(path) {
            kprintln!("{}: {}: {}", cmd, path, e);
        }
    }
}

/// What `parent` is, as a Level. Err carries a printable message.
fn level_at(path: &str) -> core::result::Result<Level, String> {
    let a = abs(path).map_err(|e| format!("{}", e))?;
    let st = file::stat(&a).map_err(|e| format!("{}", e))?;
    Ok(level_of(&a, st.kind))
}

// ---- creating and removing ----------------------------------------------------

/// newcluster / newpkg / newfldr: the new object's level must fit its parent.
fn make(args: &[&str], cmd: &str, want: Level) {
    if args.is_empty() {
        kprintln!("usage: {} <name...>", cmd);
        return;
    }
    let (need, hint) = match want {
        Level::Cluster => (
            Level::Container,
            "a Cluster goes directly inside a Container",
        ),
        Level::Package => (Level::Cluster, "a Package goes inside a Cluster"),
        Level::Folder => (Level::Package, "a Folder goes inside a Package"),
        _ => return,
    };
    for path in args {
        if let Err(msg) = create_level(path, want, need, hint) {
            kprintln!("{}: {}: {}", cmd, path, msg);
        }
    }
}

fn create_level(
    path: &str,
    want: Level,
    need: Level,
    hint: &str,
) -> core::result::Result<(), String> {
    let a = abs(path).map_err(|e| format!("{}", e))?;
    let (parent, _) = split_parent(&a).map_err(|e| format!("{}", e))?;
    let have = level_at(parent)?;
    if have != need {
        return Err(format!(
            "cannot create a {} inside a {} ({})",
            want.name(),
            have.name(),
            hint
        ));
    }
    file::mkdir(&a, 0o755).map_err(|e| format!("{}", e))
}

/// newfl: an Item goes inside a Package (directly) or inside a Folder.
fn new_item(args: &[&str]) {
    if args.is_empty() {
        kprintln!("usage: newfl <name...>");
        return;
    }
    for path in args {
        let result = (|| -> core::result::Result<(), String> {
            let a = abs(path).map_err(|e| format!("{}", e))?;
            let (parent, _) = split_parent(&a).map_err(|e| format!("{}", e))?;
            let have = level_at(parent)?;
            if have != Level::Package && have != Level::Folder {
                return Err(format!(
                    "cannot create an Item inside a {} (an Item goes inside a Package or a Folder)",
                    have.name()
                ));
            }
            let fd = file::open(&a, O_CREAT | O_WRONLY, 0o644).map_err(|e| format!("{}", e))?;
            file::close(fd).map_err(|e| format!("{}", e))
        })();
        if let Err(msg) = result {
            kprintln!("newfl: {}: {}", path, msg);
        }
    }
}

/// rm: removes an Item, or an empty Folder, Package or Cluster.
fn remove(path: &str) -> Result<()> {
    let st = file::stat(path)?;
    if st.kind == FileType::Directory {
        file::rmdir(path)
    } else {
        file::unlink(path)
    }
}

// ---- navigation ------------------------------------------------------------------

fn cd(args: &[&str]) {
    let target = match args.first() {
        Some(path) => String::from(*path),
        None => filesystem::home_dir(),
    };
    if let Err(e) = file::chdir(&target) {
        kprintln!("cd: {}: {}", target, e);
    }
}

fn pwd() {
    let cwd = file::getcwd();
    match level_at(&cwd) {
        Ok(level) => kprintln!("{}  [{}]", cwd, level.name()),
        Err(_) => kprintln!("{}", cwd),
    }
}

// ---- ls ------------------------------------------------------------------------------

fn ls(args: &[&str]) {
    let mut opts = Opts::default();
    let mut paths: Vec<&str> = Vec::new();

    for &arg in args {
        if arg.len() > 1 && arg.starts_with('-') {
            for c in arg[1..].chars() {
                match c {
                    'l' => opts.long = true,
                    'h' => opts.human = true,
                    '1' => {} // one entry per line is now the default
                    'r' => opts.reverse = true,
                    't' => opts.by_time = true,
                    'S' => opts.by_size = true,
                    _ => {
                        kprintln!("ls: invalid option -- '{}'", c);
                        kprintln!("usage: ls [-lh1rtS] [path...]");
                        return;
                    }
                }
            }
        } else {
            paths.push(arg);
        }
    }
    if paths.is_empty() {
        paths.push(".");
    }

    let multi = paths.len() > 1;
    for (i, path) in paths.iter().enumerate() {
        if multi && i > 0 {
            kprintln!();
        }
        ls_one(path, opts);
    }
}

fn ls_one(path: &str, opts: Opts) {
    let a = match abs(path) {
        Ok(a) => a,
        Err(e) => {
            kprintln!("ls: cannot access '{}': {}", path, e);
            return;
        }
    };
    let st = match file::stat(&a) {
        Ok(st) => st,
        Err(e) => {
            kprintln!("ls: cannot access '{}': {}", path, e);
            return;
        }
    };
    let level = level_of(&a, st.kind);

    // A single Item: just show that one entry.
    if st.kind != FileType::Directory {
        let entry = Entry {
            name: String::from(path),
            level,
            st: Some(st),
        };
        print_entries(&[entry], opts);
        return;
    }

    let listing = match file::readdir(&a) {
        Ok(l) => l,
        Err(e) => {
            kprintln!("ls: cannot open '{}': {}", path, e);
            return;
        }
    };

    // Header: what we are looking inside of.
    kprint!("[");
    vga::set_color(level.color());
    kprint!("{}", level.name());
    vga::set_color(0x0F);
    kprintln!("] {}", a);

    // stat every entry once, so sorting and -l don't hit the filesystem twice
    let mut entries: Vec<Entry> = listing
        .into_iter()
        .map(|e| {
            let child = join(&a, &e.name);
            let st = file::stat(&child).ok();
            let kind = st.as_ref().map_or(e.kind, |s| s.kind);
            Entry {
                level: level_of(&child, kind),
                name: e.name,
                st,
            }
        })
        .collect();

    entries.sort_by(|x, y| compare(x, y, opts));
    if opts.reverse {
        entries.reverse();
    }
    print_entries(&entries, opts);
}

/// Position of a level when sorting: containers of things first, Items last.
fn rank(l: Level) -> u8 {
    match l {
        Level::Container => 0,
        Level::Cluster => 1,
        Level::Package => 2,
        Level::Folder => 3,
        Level::Item => 4,
    }
}

fn compare(a: &Entry, b: &Entry, opts: Opts) -> Ordering {
    let by_name = || {
        a.name
            .bytes()
            .map(|c| c.to_ascii_lowercase())
            .cmp(b.name.bytes().map(|c| c.to_ascii_lowercase()))
    };
    if opts.by_time {
        let t = |e: &Entry| e.st.as_ref().map_or(0, |s| s.mtime);
        return t(b).cmp(&t(a)).then_with(by_name);
    }
    if opts.by_size {
        let s = |e: &Entry| e.st.as_ref().map_or(0, |s| s.size);
        return s(b).cmp(&s(a)).then_with(by_name);
    }
    rank(a.level).cmp(&rank(b.level)).then_with(by_name)
}

fn print_entries(entries: &[Entry], opts: Opts) {
    if entries.is_empty() {
        kprintln!("(empty)");
        return;
    }

    if opts.long {
        // Items show their size; everything else is a holder, so size is "-".
        let sizes: Vec<String> = entries
            .iter()
            .map(|e| match (&e.st, e.level) {
                (Some(s), Level::Item) => fmt_size(s.size, opts.human),
                (Some(_), _) => String::from("-"),
                (None, _) => String::from("?"),
            })
            .collect();
        let size_w = sizes.iter().map(|s| s.len()).max().unwrap_or(1);

        for (e, size) in entries.iter().zip(&sizes) {
            vga::set_color(e.level.color());
            kprint!("{:<9} ", e.level.name());
            vga::set_color(0x0F);
            match &e.st {
                Some(s) => kprint!(
                    "{:<5} {:>sw$} {} ",
                    owner(s.uid),
                    size,
                    time::Stamp(s.mtime),
                    sw = size_w
                ),
                None => kprint!("{:<5} {:>sw$} {:<16} ", "?", "?", "?", sw = size_w),
            }
            vga::set_color(e.level.color());
            kprintln!("{}", e.name);
            vga::set_color(0x0F);
        }
        return;
    }

    // One line per entry, "<Level>: <name>", so the kind is always spelled out.
    for e in entries {
        vga::set_color(e.level.color());
        kprint!("{}: ", e.level.name());
        vga::set_color(0x0F);
        kprintln!("{}", e.name);
    }
}

fn owner(uid: u32) -> String {
    if uid == 0 {
        String::from("root")
    } else {
        format!("{}", uid)
    }
}

fn fmt_size(bytes: u64, human: bool) -> String {
    if !human {
        return format!("{}", bytes);
    }
    const UNITS: [&str; 4] = ["B", "K", "M", "G"];
    let mut whole = bytes;
    let mut rem = 0;
    let mut u = 0;
    while whole >= 1024 && u < UNITS.len() - 1 {
        rem = (whole % 1024) * 10 / 1024; // one decimal digit
        whole /= 1024;
        u += 1;
    }
    if u == 0 {
        format!("{}B", whole)
    } else {
        format!("{}.{}{}", whole, rem, UNITS[u])
    }
}

// ---- reading and writing Items ----------------------------------------------------------

fn cat(args: &[&str]) {
    if args.is_empty() {
        kprintln!("usage: cat <item...>");
        return;
    }
    for path in args {
        let fd = match file::open(path, O_RDONLY, 0) {
            Ok(fd) => fd,
            Err(e) => {
                kprintln!("cat: {}: {}", path, e);
                continue;
            }
        };
        let mut buf = [0u8; 512];
        let mut last = b'\n';
        loop {
            match file::read(fd, &mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    for &b in &buf[..n] {
                        match b {
                            b'\n' | 0x20..=0x7E => vga::put_byte(b),
                            b'\t' => kprint!("    "),
                            _ => vga::put_byte(b'.'),
                        }
                    }
                    last = buf[n - 1];
                }
                Err(e) => {
                    kprintln!("cat: {}: {}", path, e);
                    break;
                }
            }
        }
        if last != b'\n' {
            kprintln!();
        }
        let _ = file::close(fd);
    }
}

fn write(args: &[&str]) {
    let append = args.first() == Some(&"-a");
    let rest = if append { &args[1..] } else { args };
    if rest.len() < 2 {
        kprintln!("usage: write [-a] <item> <text...>");
        return;
    }
    let mut text = String::new();
    for (i, word) in rest[1..].iter().enumerate() {
        if i > 0 {
            text.push(' ');
        }
        text.push_str(word);
    }
    text.push('\n');

    let flags = O_WRONLY | O_CREAT | if append { O_APPEND } else { O_TRUNC };
    let result = file::open(rest[0], flags, 0o644).and_then(|fd| {
        let written = file::write(fd, text.as_bytes());
        let _ = file::close(fd);
        written
    });
    match result {
        Ok(n) => kprintln!("wrote {} bytes to {}", n, rest[0]),
        Err(e) => kprintln!("write: {}: {}", rest[0], e),
    }
}

// ---- information ------------------------------------------------------------------------------

fn stat(args: &[&str]) {
    let Some(&path) = args.first() else {
        kprintln!("usage: stat <path>");
        return;
    };
    let a = match abs(path) {
        Ok(a) => a,
        Err(e) => {
            kprintln!("stat: {}: {}", path, e);
            return;
        }
    };
    match file::stat(&a) {
        Ok(st) => {
            let level = level_of(&a, st.kind);
            kprintln!("  path: {}", a);
            kprint!("  type: ");
            vga::set_color(level.color());
            kprintln!("{}", level.name());
            vga::set_color(0x0F);
            if level == Level::Item {
                kprintln!("  size: {} bytes", st.size);
            } else {
                kprintln!("  holds: {} entries", st.size);
            }
            kprintln!("  id: {}", st.ino);
            kprintln!(
                "  owner: {}    group: {}    mode: {:04o}",
                st.uid,
                st.gid,
                st.mode
            );
            kprintln!("  modified: {}", time::Stamp(st.mtime));
        }
        Err(e) => kprintln!("stat: {}: {}", path, e),
    }
}

fn df() {
    kprintln!(
        "{:<9} {:<8} {:>9} {:>9}  {}",
        "TYPE",
        "DEVICE",
        "USED",
        "TOTAL",
        "MOUNTED ON"
    );
    for (path, device, root) in mount::roots() {
        if let Ok((used, total)) = root.usage() {
            kprintln!(
                "{:<9} {:<8} {:>6} KB {:>6} KB  {}",
                "Container",
                device,
                used / 1024,
                total / 1024,
                path
            );
        }
    }
}

fn mounts() {
    for (path, fstype, device) in mount::list() {
        let what = if fstype == "zodiacfs" {
            "Container"
        } else {
            "Virtual"
        };
        kprintln!("{:<8} {:<9} {} ({})", device, what, path, fstype);
    }
}

fn lsblk() {
    kprintln!("{:<6} {:>10} {:>12}", "NAME", "SIZE", "BLOCKS");
    for disk in storage::list() {
        let bytes = disk.block_count() * disk.block_size() as u64;
        kprintln!(
            "{:<6} {:>7} MiB {:>12}",
            disk.name(),
            bytes / (1024 * 1024),
            disk.block_count()
        );
    }
}
