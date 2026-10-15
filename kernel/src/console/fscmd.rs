use crate::base::error::Result;
use crate::drivers::storage;
use crate::filesystem::file::{self, O_APPEND, O_CREAT, O_RDONLY, O_TRUNC, O_WRONLY};
use crate::filesystem::{self, FileType, Stat, mount};
use crate::{kprint, kprintln, time, vga};
use alloc::{format, string::String};

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
    let long = args.contains(&"-l");
    let path = args
        .iter()
        .copied()
        .find(|a| !a.starts_with('-'))
        .unwrap_or(".");

    let st = match file::stat(path) {
        Ok(st) => st,
        Err(e) => {
            kprintln!("ls: {}: {}", path, e);
            return;
        }
    };
    if st.kind != FileType::Directory {
        print_long(path, &st);
        return;
    }

    let mut entries = match file::readdir(path) {
        Ok(entries) => entries,
        Err(e) => {
            kprintln!("ls: {}: {}", path, e);
            return;
        }
    };
    entries.sort_by(|a, b| a.name.cmp(&b.name));

    let cell = entries.iter().map(|e| e.name.len()).max().unwrap_or(0) + 2;
    let width = vga::width();
    let mut x = 0;
    for entry in &entries {
        let is_dir = entry.kind == FileType::Directory;
        if long {
            if let Ok(st) = file::stat(&join(path, &entry.name)) {
                print_long(&entry.name, &st);
            }
            continue;
        }
        if x + cell > width {
            kprintln!();
            x = 0;
        }
        vga::set_color(if is_dir { 0x0B } else { 0x0F });
        kprint!("{:<width$}", entry.name, width = cell);
        vga::set_color(0x0F);
        x += cell;
    }
    if !long && x != 0 {
        kprintln!();
    }
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
