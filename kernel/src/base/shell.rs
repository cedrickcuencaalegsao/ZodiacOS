use crate::base::zpm;
use crate::{arch, ata, fs, keyboard, time, user, vga};
use crate::{kprint, kprintln};
use core::sync::atomic::{AtomicU16, Ordering::Relaxed};

/// Current folder (fs::ROOT = "/").
static CWD: AtomicU16 = AtomicU16::new(fs::ROOT);

fn cwd() -> fs::Id {
    CWD.load(Relaxed)
}

pub fn run() -> ! {
    let mut buf = [0u8; 128];

    loop {
        vga::set_color(0x0A);
        kprint!("zodiac");
        if cwd() != fs::ROOT {
            vga::set_color(0x0B);
            kprint!(":");
            print_path(cwd(), 0);
        }
        vga::set_color(0x0F);
        kprint!("> ");

        let mut len = 0usize;
        loop {
            match keyboard::wait_char() {
                b'\n' => {
                    vga::put_byte(b'\n');
                    break;
                }
                8 => {
                    if len > 0 {
                        len -= 1;
                        vga::backspace();
                    }
                }
                c @ 0x20..=0x7E => {
                    if len < buf.len() {
                        buf[len] = c;
                        len += 1;
                        vga::put_byte(c);
                    }
                }
                _ => {}
            }
        }

        let line = core::str::from_utf8(&buf[..len]).unwrap_or("");
        execute(line);
    }
}

fn print_path(id: fs::Id, depth: u8) {
    if id == fs::ROOT || depth > 16 {
        return;
    }
    if let Ok(e) = fs::get(id) {
        print_path(e.parent, depth + 1);
        kprint!("/{}", e.name());
    }
}

fn execute(line: &str) {
    let mut parts = [""; 8];
    let mut argc = 0;
    for tok in line.split_ascii_whitespace() {
        if argc < parts.len() {
            parts[argc] = tok;
            argc += 1;
        }
    }
    if argc == 0 {
        return;
    }
    let args = &parts[1..argc];

    match parts[0] {
        "help" => help(),
        "clear" => vga::clear(),
        "echo" => {
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    kprint!(" ");
                }
                kprint!("{}", a);
            }
            kprintln!();
        }
        "about" | "version" => kprintln!("ZodiacOS 0.1.0 (x86_64)"),
        "reboot" | "restart" => {
            vga::set_color(0x0E);
            kprintln!("Restarting ZodiacOS...");
            arch::sleep_ms(800);
            arch::reboot();
        }
        "shutdown" | "poweroff" => {
            vga::set_color(0x0E);
            kprintln!("Shutting down ZodiacOS...");
            arch::sleep_ms(800);
            kprintln!("It is now safe to turn off your computer.");
            arch::shutdown();
        }
        "zpm" => zpm::run(args),

        // ---- Drives and files ----
        "drives" => cmd_drives(),
        "format" => match fs::format() {
            Ok(()) => {
                CWD.store(fs::ROOT, Relaxed);
                kprintln!("hdb formatted (ZDFS)");
            }
            Err(e) => kprintln!("format: {}", e),
        },
        "ls" => cmd_ls(),
        "pwd" => {
            if cwd() == fs::ROOT {
                kprint!("/");
            } else {
                print_path(cwd(), 0);
            }
            kprintln!();
        }
        "cd" => cmd_cd(args),
        "newfl" => cmd_new(args, false),
        "newfldr" => cmd_new(args, true),
        "cat" => match args.first() {
            Some(name) => cmd_cat(name),
            None => kprintln!("usage: cat <file>"),
        },
        "write" => cmd_write(args),
        "rm" => match args.first() {
            Some(name) => match fs::remove(name, cwd()) {
                Ok(()) => kprintln!("removed {}", name),
                Err(e) => kprintln!("rm: {}: {}", name, e),
            },
            None => kprintln!("usage: rm <file or empty folder>"),
        },
        "df" => match fs::usage() {
            Ok((used, total)) => kprintln!("{} KB used of {} KB", used / 2, total / 2),
            Err(e) => kprintln!("df: {}", e),
        },
        "whoami" => {
            let u = user::get();
            kprintln!("{}", user::as_str(&u));
        }
        "login" => match args.first() {
            Some(name) => match user::set(name) {
                Ok(()) => kprintln!("now acting as {}", name),
                Err(e) => kprintln!("login: {}", e),
            },
            None => kprintln!("usage: login <name>"),
        },

        other => {
            // Installed packages work as commands
            if !zpm::launch(other, args) {
                kprintln!("{}: command not found (try 'help')", other);
            }
        }
    }
}

fn help() {
    kprintln!("System:");
    kprintln!("  whoami              show current user");
    kprintln!("  login <name>        switch user (sets the author of new files)");
    kprintln!("  help                show this list");
    kprintln!("  clear               clear the screen");
    kprintln!("  echo <text>         print text");
    kprintln!("  about               show OS version");
    kprintln!("  reboot              restart the machine (alias: restart)");
    kprintln!("  shutdown            power off (alias: poweroff)");
    kprintln!("  zpm <command>       package manager (run 'zpm' for usage)");
    kprintln!("Drives and files:");
    kprintln!("  drives              list attached disks");
    kprintln!("  format              format hdb (erases everything)");
    kprintln!("  ls                  list files and folders here");
    kprintln!("  pwd                 show current folder");
    kprintln!("  cd <folder>         enter a folder (cd .. up, cd / root)");
    kprintln!("  newfl <name...>     create empty file(s)");
    kprintln!("  newfldr <name...>   create folder(s)");
    kprintln!("  write <file> <text> write text to a file");
    kprintln!("  cat <file>          show a file");
    kprintln!("  rm <name>           delete a file or empty folder");
    kprintln!("  df                  disk usage");
    kprintln!("Installed packages can be run by name.");
}

// ---- File commands ----------------------------------------------------------
fn cmd_new(args: &[&str], is_dir: bool) {
    let cmd = if is_dir { "newfldr" } else { "newfl" };
    if args.is_empty() {
        kprintln!("usage: {} <name...>", cmd);
        return;
    }
    for name in args {
        let result = if is_dir {
            fs::mkdir(name, cwd())
        } else {
            fs::create_file(name, cwd())
        };
        match result {
            Ok(()) => {
                vga::set_color(0x0A);
                kprintln!(
                    "created {} {}",
                    if is_dir { "folder" } else { "file" },
                    name
                );
                vga::set_color(0x0F);
            }
            Err(e) => kprintln!("{}: {}: {}", cmd, name, e),
        }
    }
}

fn cmd_cd(args: &[&str]) {
    match args.first().copied().unwrap_or("/") {
        "/" => CWD.store(fs::ROOT, Relaxed),
        "." => {}
        ".." => {
            if cwd() != fs::ROOT {
                match fs::get(cwd()) {
                    Ok(e) => CWD.store(e.parent, Relaxed),
                    Err(e) => kprintln!("cd: {}", e),
                }
            }
        }
        name => match fs::lookup(name, cwd()) {
            Ok(e) if e.is_dir => CWD.store(e.id(), Relaxed),
            Ok(_) => kprintln!("cd: {}: not a folder", name),
            Err(e) => kprintln!("cd: {}: {}", name, e),
        },
    }
}

fn cmd_ls() {
    vga::set_color(0x07);
    kprintln!(
        "{:<20} {:<4} {:<8} {:>7} {:<16} {:<16}",
        "NAME",
        "TYPE",
        "AUTHOR",
        "SIZE",
        "CREATED",
        "MODIFIED"
    );

    let mut count = 0;
    let result = fs::list(cwd(), |e| {
        // Type: DIR, or the file extension (up to 4 chars), or FILE
        let kind = if e.is_dir {
            "DIR"
        } else {
            match e.name().rsplit_once('.') {
                Some((_, ext)) if !ext.is_empty() => &ext[..ext.len().min(4)],
                _ => "FILE",
            }
        };

        if e.is_dir {
            vga::set_color(0x0B);
            kprintln!(
                "{:<20} {:<4} {:<8} {:>7} {} {}",
                e.name(),
                kind,
                e.author(),
                "-",
                time::Stamp(e.created),
                time::Stamp(e.modified)
            );
        } else {
            vga::set_color(0x0F);
            kprintln!(
                "{:<20} {:<4} {:<8} {:>6}B {} {}",
                e.name(),
                kind,
                e.author(),
                e.size,
                time::Stamp(e.created),
                time::Stamp(e.modified)
            );
        }
        count += 1;
    });

    vga::set_color(0x0F);
    match result {
        Ok(()) => kprintln!("{} item(s)", count),
        Err(e) => kprintln!("ls: {}", e),
    }
}

fn cmd_cat(name: &str) {
    let result = fs::read(name, cwd(), |chunk| {
        for &b in chunk {
            match b {
                b'\n' | 0x20..=0x7E => vga::put_byte(b),
                _ => vga::put_byte(b'.'),
            }
        }
    });
    match result {
        Ok(()) => kprintln!(),
        Err(e) => kprintln!("cat: {}: {}", name, e),
    }
}

fn cmd_write(args: &[&str]) {
    if args.len() < 2 {
        kprintln!("usage: write <file> <text...>");
        return;
    }
    let mut buf = [0u8; 128];
    let mut len = 0;
    for (i, word) in args[1..].iter().enumerate() {
        if i > 0 && len < buf.len() {
            buf[len] = b' ';
            len += 1;
        }
        for &b in word.as_bytes() {
            if len < buf.len() {
                buf[len] = b;
                len += 1;
            }
        }
    }
    if len < buf.len() {
        buf[len] = b'\n';
        len += 1;
    }
    match fs::write(args[0], cwd(), &buf[..len]) {
        Ok(()) => kprintln!("wrote {} bytes to {}", len, args[0]),
        Err(e) => kprintln!("write: {}: {}", args[0], e),
    }
}

fn cmd_drives() {
    kprintln!("{:<6} {:>7}  {}", "DRIVE", "SIZE", "MODEL");
    let names = ["hda", "hdb", "hdc", "hdd"];
    let mut found = 0;
    for id in 0..4u8 {
        if let Some(d) = ata::identify(id) {
            found += 1;
            let model = core::str::from_utf8(&d.model).unwrap_or("?").trim_end();
            kprintln!(
                "{:<6} {:>4} MB  {}",
                names[id as usize],
                d.sectors / 2048,
                model
            );
        }
    }
    if found == 0 {
        kprintln!("no drives found");
    }
}
