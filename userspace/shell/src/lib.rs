//! The ZodiacOS shell. Knows nothing about drawing or the kernel:
//! it consumes key events and talks through `Console` and `Fs`.
#![no_std]
extern crate alloc;

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use zodiac_abi::{Action, Console, DirEntry, Fs, Key};

const HOME: &str = "/root";

/// (name, description)
const PACKAGES: [(&str, &str); 3] = [
    ("sysinfo", "System info with the Orion constellation"),
    ("hello", "Hello world demo"),
    ("calc", "Simple calculator"),
];

pub struct Shell {
    user: String,
    cwd: String,
    line: String,
    history: Vec<String>,
    hist_pos: Option<usize>,
    installed: Vec<String>,
}

impl Shell {
    pub fn new() -> Self {
        Shell {
            user: "root".to_string(),
            cwd: HOME.to_string(),
            line: String::new(),
            history: Vec::new(),
            hist_pos: None,
            installed: alloc::vec!["sysinfo".to_string()],
        }
    }

    /// Print the banner and the first prompt.
    pub fn start(&mut self, out: &mut dyn Console) {
        out.write_str("ZodiacOS 0.1.0 (x86_64)\n");
        out.write_str("Type 'help' for a list of commands.\n\n");
        self.prompt(out);
    }

    pub fn prompt_string(&self) -> String {
        let loc = if self.cwd == HOME {
            "/~".to_string()
        } else if let Some(rest) = self.cwd.strip_prefix(&format!("{}/", HOME)) {
            format!("/~/{}", rest)
        } else {
            self.cwd.clone()
        };
        format!("{}@zodiacos{}> ", self.user, loc)
    }

    fn prompt(&self, out: &mut dyn Console) {
        out.write_str(&self.prompt_string());
    }

    pub fn on_key(&mut self, key: Key, out: &mut dyn Console, fs: &mut dyn Fs) -> Action {
        match key {
            Key::Char(c) => {
                self.hist_pos = None;
                self.line.push(c);
                let mut b = [0u8; 4];
                out.write_str(c.encode_utf8(&mut b));
                Action::None
            }
            Key::Backspace => {
                if self.line.pop().is_some() {
                    out.write_str("\x08");
                }
                Action::None
            }
            Key::Enter => {
                out.write_str("\n");
                let raw = core::mem::take(&mut self.line);
                let line = raw.trim().to_string();
                self.hist_pos = None;
                if !line.is_empty() {
                    self.history.push(line.clone());
                }
                let act = self.execute(&line, out, fs);
                if act == Action::None {
                    self.prompt(out);
                }
                act
            }
            Key::Up => {
                if self.history.is_empty() {
                    return Action::None;
                }
                let pos = match self.hist_pos {
                    None => self.history.len() - 1,
                    Some(0) => 0,
                    Some(p) => p - 1,
                };
                self.hist_pos = Some(pos);
                let l = self.history[pos].clone();
                self.recall(out, l);
                Action::None
            }
            Key::Down => {
                if let Some(p) = self.hist_pos {
                    if p + 1 < self.history.len() {
                        self.hist_pos = Some(p + 1);
                        let l = self.history[p + 1].clone();
                        self.recall(out, l);
                    } else {
                        self.hist_pos = None;
                        self.recall(out, String::new());
                    }
                }
                Action::None
            }
            Key::Tab => Action::None,
        }
    }

    /// Replace the text being edited with `new`.
    fn recall(&mut self, out: &mut dyn Console, new: String) {
        for _ in 0..self.line.chars().count() {
            out.write_str("\x08");
        }
        out.write_str(&new);
        self.line = new;
    }

    fn resolve(&self, arg: &str) -> String {
        let base = if arg == "~" || arg.starts_with("~/") {
            format!("{}{}", HOME, &arg[1..])
        } else if arg.starts_with('/') {
            arg.to_string()
        } else {
            format!("{}/{}", self.cwd, arg)
        };
        let mut stack: Vec<&str> = Vec::new();
        for seg in base.split('/') {
            match seg {
                "" | "." => {}
                ".." => {
                    stack.pop();
                }
                s => stack.push(s),
            }
        }
        if stack.is_empty() {
            return "/".to_string();
        }
        let mut out = String::new();
        for s in stack {
            out.push('/');
            out.push_str(s);
        }
        out
    }

    fn execute(&mut self, line: &str, out: &mut dyn Console, fs: &mut dyn Fs) -> Action {
        let parts: Vec<&str> = line.split_whitespace().collect();
        let Some(&cmd) = parts.first() else {
            return Action::None;
        };
        let args = &parts[1..];

        match cmd {
            "help" => out.write_str(
                "Commands:\n\
                 \x20 help              show this list\n\
                 \x20 clear             clear the screen\n\
                 \x20 echo <text>       print text\n\
                 \x20 whoami            current user\n\
                 \x20 version           OS version\n\
                 \x20 pwd               print working directory\n\
                 \x20 cd <dir>          change directory\n\
                 \x20 ls [dir]          list items (author, created, modified)\n\
                 \x20 newfl <name>      create a new file\n\
                 \x20 newfldr <name>    create a new folder\n\
                 \x20 zpm <list|install|remove> [pkg]\n\
                 \x20 sysinfo           system info (zpm package)\n\
                 \x20 shutdown          power off\n\
                 \x20 restart           reboot\n",
            ),
            "clear" => out.clear(),
            "echo" => {
                out.write_str(&args.join(" "));
                out.write_str("\n");
            }
            "whoami" => {
                out.write_str(&self.user);
                out.write_str("\n");
            }
            "version" => out.write_str("ZodiacOS 0.1.0 (x86_64)\n"),
            "pwd" => {
                out.write_str(&self.cwd);
                out.write_str("\n");
            }
            "cd" => {
                let target = match args.first() {
                    Some(a) => self.resolve(a),
                    None => HOME.to_string(),
                };
                if fs.is_dir(&target) {
                    self.cwd = target;
                } else {
                    out.write_str("cd: no such directory\n");
                }
            }
            "ls" => {
                let path = match args.first() {
                    Some(a) => self.resolve(a),
                    None => self.cwd.clone(),
                };
                match fs.list(&path) {
                    Ok(mut entries) => self.print_listing(out, &mut entries),
                    Err(e) => out.write_str(&format!("ls: {}\n", e.msg())),
                }
            }
            "newfl" | "newfldr" => match args.first() {
                None => out.write_str(&format!("usage: {} <name>\n", cmd)),
                Some(a) => {
                    let path = self.resolve(a);
                    let res = if cmd == "newfl" {
                        fs.new_file(&path, &self.user)
                    } else {
                        fs.new_dir(&path, &self.user)
                    };
                    if let Err(e) = res {
                        out.write_str(&format!("{}: {}\n", cmd, e.msg()));
                    }
                }
            },
            "zpm" => self.zpm(args, out),
            "sysinfo" => {
                if self.installed.iter().any(|p| p == "sysinfo") {
                    out.write_str(
                        "      *        *\n\
                         \x20      \\      /       root@zodiacos\n\
                         \x20  *---*--*--*---*    OS: ZodiacOS 0.1.0\n\
                         \x20      /  |  \\       Arch: x86_64\n\
                         \x20  *--*---*---*--*    Shell: zodiac-shell\n\
                         \x20      \\  |  /        Constellation: Orion\n\
                         \x20       *   *\n",
                    );
                } else {
                    out.write_str("sysinfo: not installed (try: zpm install sysinfo)\n");
                }
            }
            "shutdown" => {
                out.write_str("Shutting down...\n");
                return Action::Shutdown;
            }
            "restart" => {
                out.write_str("Restarting...\n");
                return Action::Restart;
            }
            other => out.write_str(&format!("{}: command not found\n", other)),
        }
        Action::None
    }

    fn print_listing(&self, out: &mut dyn Console, entries: &mut Vec<DirEntry>) {
        if entries.is_empty() {
            return;
        }
        entries.sort_by(|a, b| (!a.is_dir, &a.name).cmp(&(!b.is_dir, &b.name)));
        out.write_str("TYPE  AUTHOR  CREATED   MODIFIED  NAME\n");
        for e in entries.iter() {
            out.write_str(&format!(
                "{:<5} {:<7} {}  {}  {}\n",
                if e.is_dir { "dir" } else { "file" },
                e.author,
                hms(e.created),
                hms(e.modified),
                e.name
            ));
        }
    }

    fn zpm(&mut self, args: &[&str], out: &mut dyn Console) {
        match args.first().copied() {
            Some("list") => {
                for (name, desc) in PACKAGES.iter() {
                    let status = if self.installed.iter().any(|p| p == name) {
                        "installed"
                    } else {
                        "available"
                    };
                    out.write_str(&format!("{:<9} {:<10} {}\n", name, status, desc));
                }
            }
            Some("install") => match args.get(1) {
                Some(&p) if PACKAGES.iter().any(|(n, _)| *n == p) => {
                    if self.installed.iter().any(|i| i == p) {
                        out.write_str(&format!("zpm: {} is already installed\n", p));
                    } else {
                        self.installed.push(p.to_string());
                        out.write_str(&format!("zpm: installed {}\n", p));
                    }
                }
                Some(p) => out.write_str(&format!("zpm: unknown package '{}'\n", p)),
                None => out.write_str("usage: zpm install <pkg>\n"),
            },
            Some("remove") => match args.get(1) {
                Some(&p) => {
                    if let Some(i) = self.installed.iter().position(|x| x == p) {
                        self.installed.remove(i);
                        out.write_str(&format!("zpm: removed {}\n", p));
                    } else {
                        out.write_str(&format!("zpm: {} is not installed\n", p));
                    }
                }
                None => out.write_str("usage: zpm remove <pkg>\n"),
            },
            _ => out.write_str("usage: zpm <list|install|remove> [pkg]\n"),
        }
    }
}

fn hms(s: u64) -> String {
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}