use crate::drivers::storage;
use crate::filesystem::file::{self, O_APPEND, O_CREAT, O_RDONLY, O_TRUNC, O_WRONLY};
use crate::filesystem::{self, FileType, Stat, mount};
use crate::{kprint, kprintln, time, vga};
use alloc::{format, string::String, vec::Vec};
use core::cmp::Ordering;
use errors::Result;

#[derive(Default, Clone, Copy)]
struct Opts {
    long: bool,
    all: bool,
    human: bool,
    classify: bool,
    one: bool,
    reverse: bool,
    by_time: bool,
    by_size: bool,
}

struct Item {
    name: String,
    kind: FileType,
    st: Option<Stat>,
}

pub fn run(cmd: &str, args: &[&str]) {
    match cmd {
        "ls" => ls(args),
        "cd" => cd(args),
        "pwd" => kprintln!("{}", file::getcwd()),
        "cat" => cat(args),
        "mkdir" | "newfldr" => each(args, cmd, |p| file::mkdir(p, 0o755)),
        "touch" | "newfl" => each(args, cmd, touch),
        "rm" => each(args, cmd, file::unlink),
        "rmdir" => each(args, cmd, file::rmdir),
        "write" => write(args),
        "stat" => stat(args),
        "mount" => mounts(),
        "lsblk" | "drives" => lsblk(),
        _ => {}
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

fn touch(path: &str) -> Result<()> {
    let fd = file::open(path, O_CREAT | O_WRONLY, 0o644)?;
    file::close(fd)
}

fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{}{}", dir, name)
    } else {
        format!("{}/{}", dir, name)
    }
}

fn mode_string(kind: FileType, mode: u16) -> String {
    let mut s = String::with_capacity(10);
    s.push(match kind {
        FileType::Directory => 'd',
        FileType::CharDevice => 'c',
        FileType::File => '-',
    });
    for shift in [6, 3, 0] {
        let bits = (mode >> shift) & 7;
        s.push(if bits & 4 != 0 { 'r' } else { '-' });
        s.push(if bits & 2 != 0 { 'w' } else { '-' });
        s.push(if bits & 1 != 0 { 'x' } else { '-' });
    }
    s
}

fn print_long(name: &str, st: &Stat) {
    if st.kind == FileType::Directory {
        vga::set_color(0x0B);
    }
    kprintln!(
        "{} {:>8} {} {}",
        mode_string(st.kind, st.mode),
        st.size,
        time::Stamp(st.mtime),
        name
    );
    vga::set_color(0x0F);
}

fn ls(args: &[&str]) {
    let mut opts = Opts::default();
    let mut paths: Vec<&str> = Vec::new();

    for &arg in args {
        if arg.len() > 1 && arg.starts_with('-') {
            for c in arg[1..].chars() {
                match c {
                    'l' => opts.long = true,
                    'a' => opts.all = true,
                    'h' => opts.human = true,
                    'F' => opts.classify = true,
                    '1' => opts.one = true,
                    'r' => opts.reverse = true,
                    't' => opts.by_time = true,
                    'S' => opts.by_size = true,
                    _ => {
                        kprintln!("ls: invalid option -- '{}'", c);
                        kprintln!("usage: ls [-lahF1rtS] [path...]");
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
        if multi {
            if i > 0 {
                kprintln!();
            }
            kprintln!("{}:", path);
        }
        ls_one(path, opts);
    }
}

fn ls_one(path: &str, opts: Opts) {
    let st = match file::stat(path) {
        Ok(st) => st,
        Err(e) => {
            kprintln!("ls: cannot access '{}': {}", path, e);
            return;
        }
    };

    // A plain file: just show that one entry.
    if st.kind != FileType::Directory {
        let item = Item {
            name: String::from(path),
            kind: st.kind,
            st: Some(st),
        };
        print_items(&[item], opts);
        return;
    }

    let entries = match file::readdir(path) {
        Ok(e) => e,
        Err(e) => {
            kprintln!("ls: cannot open '{}': {}", path, e);
            return;
        }
    };

    // stat every entry once, so sorting and -l don't hit the filesystem twice
    let mut items: Vec<Item> = entries
        .into_iter()
        .filter(|e| opts.all || !e.name.starts_with('.'))
        .map(|e| {
            let st = file::stat(&join(path, &e.name)).ok();
            Item {
                name: e.name,
                kind: e.kind,
                st,
            }
        })
        .collect();

    items.sort_by(|a, b| compare(a, b, opts));
    if opts.reverse {
        items.reverse();
    }

    print_items(&items, opts);
}

fn compare(a: &Item, b: &Item, opts: Opts) -> Ordering {
    let by_name = || {
        a.name
            .bytes()
            .map(|c| c.to_ascii_lowercase())
            .cmp(b.name.bytes().map(|c| c.to_ascii_lowercase()))
    };

    if opts.by_time {
        let t = |i: &Item| i.st.as_ref().map_or(0, |s| s.mtime);
        return t(b).cmp(&t(a)).then_with(by_name);
    }
    if opts.by_size {
        let s = |i: &Item| i.st.as_ref().map_or(0, |s| s.size);
        return s(b).cmp(&s(a)).then_with(by_name);
    }

    // default: directories first, then name (case-insensitive)
    let a_dir = a.kind == FileType::Directory;
    let b_dir = b.kind == FileType::Directory;
    b_dir.cmp(&a_dir).then_with(by_name)
}

fn display_name(item: &Item, opts: Opts) -> String {
    if opts.classify && item.kind == FileType::Directory {
        format!("{}/", item.name)
    } else {
        item.name.clone()
    }
}

fn name_color(item: &Item) -> u8 {
    if item.kind == FileType::Directory {
        0x0B
    } else {
        0x0F
    }
}

fn print_items(items: &[Item], opts: Opts) {
    if items.is_empty() {
        return;
    }

    if opts.long {
        let sizes: Vec<String> = items
            .iter()
            .map(|i| fmt_size(i.st.as_ref().map_or(0, |s| s.size), opts.human))
            .collect();
        let size_w = sizes.iter().map(|s| s.len()).max().unwrap_or(1);

        for (item, size) in items.iter().zip(&sizes) {
            match &item.st {
                Some(s) => kprint!(
                    "{} {:<5} {:>sw$} {} ",
                    mode_string(s.kind, s.mode),
                    owner(s.uid),
                    size,
                    time::Stamp(s.mtime),
                    sw = size_w
                ),
                None => kprint!(
                    "{:<10} {:<5} {:>sw$} {:<16} ",
                    "?",
                    "?",
                    "?",
                    "?",
                    sw = size_w
                ),
            }
            vga::set_color(name_color(item));
            kprintln!("{}", display_name(item, opts));
            vga::set_color(0x0F);
        }
        return;
    }

    if opts.one {
        for item in items {
            vga::set_color(name_color(item));
            kprintln!("{}", display_name(item, opts));
            vga::set_color(0x0F);
        }
        return;
    }

    // Short format: column-major grid like real ls (fills down, then across)
    let names: Vec<String> = items.iter().map(|i| display_name(i, opts)).collect();
    let cell = names.iter().map(|n| n.len()).max().unwrap_or(0) + 2;
    let cols = core::cmp::max(1, vga::width() / cell);
    let rows = (items.len() + cols - 1) / cols;

    for r in 0..rows {
        for c in 0..cols {
            let idx = c * rows + r;
            if idx >= items.len() {
                break;
            }
            vga::set_color(name_color(&items[idx]));
            kprint!("{:<w$}", names[idx], w = cell);
            vga::set_color(0x0F);
        }
        kprintln!();
    }
}

fn owner(uid: u32) -> String {
    if uid == 0 { String::from("root") } else { format!("{}", uid) }
}

fn type_label(kind: FileType) -> &'static str {
    match kind {
        FileType::Directory => "dir",
        _ => "file",
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

/// Unix seconds -> "YYYY-MM-DD HH:MM" (UTC)
fn fmt_time(ts: u64) -> String {
    if ts == 0 {
        return String::from("-");
    }
    let days = (ts / 86_400) as i64;
    let secs = ts % 86_400;

    // days since 1970-01-01 -> civil date (Howard Hinnant's algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };

    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        y,
        m,
        d,
        secs / 3600,
        (secs % 3600) / 60
    )
}

fn cd(args: &[&str]) {
    let target = match args.first() {
        Some(path) => String::from(*path),
        None => filesystem::home_dir(),
    };
    if let Err(e) = file::chdir(&target) {
        kprintln!("cd: {}: {}", target, e);
    }
}

fn cat(args: &[&str]) {
    if args.is_empty() {
        kprintln!("usage: cat <file...>");
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
        kprintln!("usage: write [-a] <file> <text...>");
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

fn stat(args: &[&str]) {
    let Some(&path) = args.first() else {
        kprintln!("usage: stat <path>");
        return;
    };
    match file::stat(path) {
        Ok(st) => {
            kprintln!("  file: {}", path);
            kprintln!(
                "  type: {}",
                match st.kind {
                    FileType::File => "regular file",
                    FileType::Directory => "directory",
                    FileType::CharDevice => "character device",
                }
            );
            kprintln!(
                "  size: {}    inode: {}    links: {}",
                st.size,
                st.ino,
                st.nlink
            );
            kprintln!(
                "  mode: {} ({:04o})    uid: {}    gid: {}",
                mode_string(st.kind, st.mode),
                st.mode,
                st.uid,
                st.gid
            );
            kprintln!("  modified: {}", time::Stamp(st.mtime));
        }
        Err(e) => kprintln!("stat: {}: {}", path, e),
    }
}

fn mounts() {
    for (path, fstype, device) in mount::list() {
        kprintln!("{} on {} type {}", device, path, fstype);
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
