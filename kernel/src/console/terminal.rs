use crate::console::netcmd;
use crate::console::{fscmd, hwcmd, memcmd, taskcmd, zpm_cmd};
use crate::filesystem::{self, file};
use crate::{arch, fs, interrupts, keyboard, user, vga};
use crate::{kprint, kprintln};
use alloc::{format, string::String};
use core::fmt;
use shared::kassert_eq;

use zpm_core::{Package, PackageKind, PackageSpace, Registry, Version};

const MB: u64 = 1024 * 1024;

pub static ZPM: spin::Mutex<Registry<128>> = spin::Mutex::new(Registry::new(PackageSpace::new(
    64 * MB,
    16 * MB,
    4 * MB,
    16 * MB,
)));

pub static CATALOG: spin::Mutex<Registry<16>> = spin::Mutex::new(Registry::new(PackageSpace::new(
    64 * MB,
    16 * MB,
    4 * MB,
    16 * MB,
)));

fn ensure_catalog() {
    let mut cat = CATALOG.lock();
    if cat.iter().next().is_some() {
        return;
    }
    if let Some(hello) = Package::new("hello", Version::new(0, 1, 0), PackageKind::App) {
        let _ = cat.install(hello);
    }
}

/// Sends `write!`/`writeln!` output to the screen.
struct ConsoleWriter;

impl fmt::Write for ConsoleWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        kprint!("{}", s);
        Ok(())
    }
}

pub fn run() -> ! {
    let mut buf = [0u8; 128];

    loop {
        print_prompt();

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

fn print_prompt() {
    let u = user::get();

    // "~" stands for the home directory
    let cwd = file::getcwd();
    let home = filesystem::home_dir();
    let shown = if cwd == home {
        String::from("/~")
    } else if let Some(rest) = cwd
        .strip_prefix(home.as_str())
        .and_then(|r| r.strip_prefix('/'))
    {
        format!("/~/{}", rest)
    } else {
        cwd.clone()
    };

    vga::set_color(0x0A); // green: username
    kprint!("{}", user::as_str(&u));
    vga::set_color(0x0F);
    kprint!("@");
    vga::set_color(0x0B); // light cyan: hostname
    kprint!("zodiacos");
    vga::set_color(if shown.starts_with("/~") { 0x0E } else { 0x09 });
    kprint!("{}", shown);
    vga::set_color(0x0F);
    kprint!("> ");
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
            arch::shutdown();
        }

        // ---- Package manager ----
        "zpm" => {
            let rest = line.strip_prefix("zpm").unwrap_or("");
            ensure_catalog();
            zpm_cmd::run(rest, &mut *ZPM.lock(), &*CATALOG.lock(), &mut ConsoleWriter);
        }
        "hello" => {
            let rest = line.strip_prefix("hello").unwrap_or("").trim();
            let installed = ZPM.lock().get("hello").is_some();
            if !installed {
                kprintln!("hello: not installed (run: zpm install hello)");
            } else {
                let name = if rest.is_empty() { "world" } else { rest };
                kprintln!("Hello, {}! Greetings from ZodiacOS.", name);
            }
        }

        // ---- Drives and files ----
        // ls, cd, pwd, cat, write, rm, mkdir, touch, rmdir, stat, mount, lsblk
        // are handled by fscmd below.
        "format" => match fs::format() {
            Ok(()) => kprintln!("hdb formatted (ZDFS)"),
            Err(e) => kprintln!("format: {}", e),
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
        "uptime" => {
            let ms = interrupts::uptime_ms();
            kprintln!(
                "up {}m {}s ({} ticks)",
                ms / 60_000,
                ms / 1000 % 60,
                interrupts::ticks()
            );
        }
        "sleep" => match args.first().and_then(|a| a.parse::<u64>().ok()) {
            Some(ms) => interrupts::sleep_ms(ms),
            None => kprintln!("usage: sleep <milliseconds>"),
        },
        "crash" => match args.first().copied() {
            Some("panic") => panic!("test panic from the shell"),
            Some("assert") => {
                let n = args.len();
                kassert_eq!(n, 99);
            }
            Some("ud") => unsafe { core::arch::asm!("ud2") },
            Some("pf") => unsafe {
                let _ = core::ptr::read_volatile(0x4000_0000 as *const u8);
            },
            Some("div") => unsafe {
                core::arch::asm!(
                    "xor edx, edx", "mov eax, 1", "xor ecx, ecx", "div ecx",
                    out("eax") _, out("ecx") _, out("edx") _
                )
            },
            Some("null") => unsafe {
                let _ = core::ptr::read_volatile(0x10 as *const u8);
            },
            Some("write") => unsafe {
                // the shell's own machine code lives in read-only pages
                core::ptr::write_volatile(execute as *const () as *mut u8, 0);
            },
            Some("bp") => unsafe { core::arch::asm!("int3") },
            Some("stack") => taskcmd::overflow_test(),
            _ => kprintln!("usage: crash <panic|assert|ud|pf|div|null|write|bp|stack>"),
        },
        "meminfo" | "memmap" | "vmem" | "memtest" | "vmtest" => memcmd::run(parts[0], args),
        "lspci" | "display" => hwcmd::run(parts[0]),
        "ps" | "threads" | "syncdemo" | "userdemo" => taskcmd::run(parts[0], args),
        "ls" | "cd" | "pwd" | "cat" | "mkdir" | "touch" | "rm" | "rmdir" | "write" | "stat"
        | "mount" | "lsblk" => fscmd::run(parts[0], args),
        "ifconfig" | "ping" | "arp" | "wifi" => netcmd::run(parts[0], args),

        // Installed packages can't be launched yet (zpm_core has no launcher).
        other => kprintln!("{}: command not found (try 'help')", other),
    }
}

// ---- Help -------------------------------------------------------------------

// VGA color attributes used by the help screen.
const HELP_HEADING: u8 = 0x0B; // light cyan
const HELP_CMD: u8 = 0x0E; // yellow
const HELP_ARGS: u8 = 0x0A; // light green
const HELP_DESC: u8 = 0x07; // light gray
const HELP_DEFAULT: u8 = 0x0F; // white (shell default)

/// Width of the "command + arguments" column.
const HELP_COL: usize = 20;

/// Prints a section heading.
fn help_heading(title: &str) {
    vga::set_color(HELP_HEADING);
    kprintln!("{}", title);
}

/// Prints one help line: command in one color, arguments in another,
/// padded to a fixed column, then the description.
/// Pass "" for `args` when the command takes none.
fn help_line(cmd: &str, args: &str, desc: &str) {
    vga::set_color(HELP_CMD);
    kprint!("  {}", cmd);

    let mut used = cmd.len();
    if !args.is_empty() {
        vga::set_color(HELP_ARGS);
        kprint!(" {}", args);
        used += 1 + args.len();
    }

    // Pad so the descriptions line up (+1 keeps a gap on long entries).
    let pad = if used < HELP_COL { HELP_COL - used } else { 1 };
    for _ in 0..pad {
        kprint!(" ");
    }

    vga::set_color(HELP_DESC);
    kprintln!("{}", desc);
}

fn help() {
    help_heading("System:");
    help_line("whoami", "", "show current user");
    help_line(
        "login",
        "<name>",
        "switch user (sets the author of new files)",
    );
    help_line("help", "", "show this list");
    help_line("clear", "", "clear the screen");
    help_line("echo", "<text>", "print text");
    help_line("about", "", "show OS version");
    help_line("reboot", "", "restart the machine (alias: restart)");
    help_line("shutdown", "", "power off (alias: poweroff)");
    help_line("zpm", "list", "package manager (run 'zpm' for usage)");
    help_line("uptime", "", "time since boot");
    help_line("sleep", "<ms>", "wait using the timer interrupt");
    help_line(
        "crash",
        "<panic|assert|ud|pf|div|null|write|bp|stack>",
        "trigger a test crash",
    );

    help_heading("Files and drives:");
    help_line("ls", "[-l] [path]", "list a directory");
    help_line("cd", "[path]", "change directory (no argument = home)");
    help_line("pwd", "", "show the current directory");
    help_line("cat", "<file...>", "print files");
    help_line("write", "[-a] <file> <text>", "write text to a file");
    help_line("mkdir", "<path...>", "create directories");
    help_line("touch", "<path...>", "create empty files");
    help_line("rm", "<path...>", "delete files");
    help_line("rmdir", "<path...>", "delete empty directories");
    help_line("stat", "<path>", "file details");
    help_line("df", "", "disk usage");
    help_line("mount", "", "list mounted filesystems");
    help_line("lsblk", "", "list disks");
    help_line("format", "", "format hdb (erases everything)");

    help_heading("Memory:");
    help_line("meminfo", "", "physical memory and heap usage");
    help_line("memmap", "", "memory map from the BIOS");
    help_line("vmem", "<hex addr>", "show how an address is mapped");
    help_line("memtest", "", "allocate with Vec, String and Box");
    help_line("vmtest", "", "build and destroy a second address space");

    help_heading("Hardware:");
    help_line("lspci", "", "list PCI devices");
    help_line("display", "", "show the screen mode");

    help_heading("Processes:");
    help_line("ps", "", "list tasks");
    help_line("threads", "", "three CPU-bound threads sharing the CPU");
    help_line("syncdemo", "", "lost updates vs atomics vs a mutex");
    help_line("userdemo", "<hello|read|kptr|priv>", "run a ring 3 program");

    help_heading("Network:");
    help_line(
        "ifconfig",
        "[set <ip> <mask> <gw>]",
        "show interfaces or set the address",
    );
    help_line("ping", "<a.b.c.d>", "send ICMP echo requests");
    help_line("arp", "", "show the ARP table");
    help_line("wifi", "[scan|connect|selftest]", "WiFi control");

    vga::set_color(HELP_DESC);
    kprintln!("Installed packages can be run by name.");
    vga::set_color(HELP_DEFAULT); // restore the shell's default color
}
