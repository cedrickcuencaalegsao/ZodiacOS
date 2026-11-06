use crate::console::netcmd;
use crate::console::{auth, fscmd, hwcmd, memcmd, taskcmd, zpm_cmd};
use crate::filesystem::{self, Level, file};
use crate::{arch, interrupts, keyboard, user, vga};
use crate::{kprint, kprintln};
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

    // start in the root directory instead of home
    fscmd::run("cd", &["/"]);

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
    vga::set_color(0x0A); // green: username
    kprint!("{}", user::as_str(&u));
    vga::set_color(0x0F);
    kprint!("@");
    vga::set_color(0x0B); // light cyan: hostname
    kprint!("zodiacos");
    vga::set_color(0x0F);
    kprint!("/");

    // [Container]/{Cluster}/Package/Folder, each part in its level's color
    let (container, rest) = filesystem::container_of(&file::getcwd());
    vga::set_color(Level::Container.color());
    kprint!("[{}]", container);
    for (i, part) in rest.split('/').filter(|p| !p.is_empty()).enumerate() {
        vga::set_color(0x0F);
        kprint!("/");
        match i {
            0 => {
                vga::set_color(Level::Cluster.color());
                kprint!("{{{}}}", part);
            }
            1 => {
                vga::set_color(Level::Package.color());
                kprint!("{}", part);
            }
            _ => {
                vga::set_color(Level::Folder.color());
                kprint!("{}", part);
            }
        }
    }
    vga::set_color(0x0F);
    kprint!("> ");
}

#[cfg(target_arch = "x86_64")]
const ARCH_NAME: &str = "x86_64";
#[cfg(target_arch = "aarch64")]
const ARCH_NAME: &str = "aarch64";

// Must be unmapped. 0x4000_0000 is RAM on QEMU's aarch64 `virt` board, so it would not fault there.
#[cfg(target_arch = "x86_64")]
const PF_ADDR: usize = 0x4000_0000;
#[cfg(target_arch = "aarch64")]
const PF_ADDR: usize = 0x0000_1000_0000_0000;

fn crash_undefined() {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::asm!("ud2")
    };
    #[cfg(target_arch = "aarch64")]
    unsafe {
        core::arch::asm!("udf #0")
    };
}

fn crash_breakpoint() {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::asm!("int3")
    };
    #[cfg(target_arch = "aarch64")]
    unsafe {
        core::arch::asm!("brk #0")
    };
}

fn crash_div() {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::asm!(
            "xor edx, edx", "mov eax, 1", "xor ecx, ecx", "div ecx",
            out("rax") _, out("rcx") _, out("rdx") _
        )
    };
    #[cfg(target_arch = "aarch64")]
    kprintln!("div: integer division by zero does not trap on aarch64 (result is 0)");
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
        "clear" | "cls" => vga::clear(),
        "echo" => {
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    kprint!(" ");
                }
                kprint!("{}", a);
            }
            kprintln!();
        }
        "about" | "version" => kprintln!("ZodiacOS 0.1.0 ({})", ARCH_NAME),
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
        "service" => crate::console::svccmd::run(args),

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

        "format" => format_cmd(args),
        "whoami" => {
            let u = user::get();
            kprintln!("{}", user::as_str(&u));
        }
        "logout" | "login" => auth::logout_cmd(),
        "exit" => match crate::security::service::exit_elevated() {
            Ok(()) => kprintln!("back to {}", user::as_str(&user::get())),
            Err(_) => auth::logout_cmd(), // not in an su: log out
        },
        "adduser" => auth::adduser_cmd(args.first().copied()),
        "user" => auth::user_cmd(args),
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
            Some("ud") => crash_undefined(),
            Some("pf") => unsafe {
                let _ = core::ptr::read_volatile(PF_ADDR as *const u8);
            },
            Some("div") => crash_div(),
            Some("null") => unsafe {
                let _ = core::ptr::read_volatile(0x10 as *const u8);
            },
            Some("write") => unsafe {
                // the shell's own machine code lives in read-only pages
                core::ptr::write_volatile(execute as *const () as *mut u8, 0);
            },
            Some("bp") => crash_breakpoint(),
            Some("stack") => taskcmd::overflow_test(),
            _ => kprintln!("usage: crash <panic|assert|ud|pf|div|null|write|bp|stack>"),
        },
        "meminfo" | "memmap" | "vmem" | "memtest" | "vmtest" => memcmd::run(parts[0], args),
        "lspci" | "display" => hwcmd::run(parts[0]),
        "ps" | "threads" | "syncdemo" | "userdemo" => taskcmd::run(parts[0], args),
        "ls" | "cd" | "pwd" | "cat" | "newcluster" | "newpkg" | "newfldr" | "newfl" | "rm"
        | "write" | "stat" | "mount" | "umount" | "lsblk" | "df" | "tree" => {
            fscmd::run(parts[0], args)
        }
        "ifconfig" | "ping" | "arp" | "wifi" => netcmd::run(parts[0], args),
        "tskmngr" => {
            use crate::console::{tskmngr, tskmngr_glue::*};
            let read = crate::drivers::keyboard::try_read_key; // WIRE
            match FbTerm::new(read) {
                Some(mut term) => tskmngr::run(&mut ZodiacPlatform, &mut term),
                None => {
                    let mut term = VgaTerm::new(read);
                    tskmngr::run(&mut ZodiacPlatform, &mut term);
                }
            }
            // WIRE: redraw/clear your fbcon here, because tskmngr painted over it
        }
        "zedit" => {
            use crate::console::zedit_glue;
            zedit_glue::run(args.first().copied());
            // zedit painted over the screen, so repaint the shell
            vga::clear();
        }

        // Installed packages can't be launched yet (zpm_core has no launcher).
        other => kprintln!("{}: command not found (try 'help')", other),
    }
}

fn format_cmd(args: &[&str]) {
    let label = args.first().copied().filter(|a| *a != "--yes");
    match (label, args.contains(&"--yes")) {
        (Some(l), true) => match filesystem::format_data(l) {
            Ok(()) => kprintln!(
                "hdb formatted as Container '{}', mounted at {}",
                l,
                filesystem::DATA_MOUNT
            ),
            Err(e) => kprintln!("format: {}", e),
        },
        (Some(l), false) => {
            kprintln!("format ERASES hdb and creates a Container named '{}'.", l);
            kprintln!("Run: format {} --yes", l);
        }
        _ => kprintln!("usage: format <name> --yes"),
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
    help_line("login", "", "switch account (alias: logout)");
    help_line("logout", "", "log out and pick an account");
    help_line("exit", "", "leave an su session, or log out");
    help_line("user", "<list|passwd>", "manage accounts (root only)");
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

    help_heading("Services:");
    help_line(
        "service",
        "<list|new|start|kill|restart|remove>",
        "background services (root for changes)",
    );

    help_heading("Files (Container > Cluster > Package > Folder > Item):");
    help_line(
        "ls",
        "[-lh1rtS] [path]",
        "list a path; shows what each entry is",
    );
    help_line(
        "tree",
        "[--name | path]",
        "show everything inside a Container, Cluster, Package or Folder",
    );
    help_line("cd", "[path]", "change path (no argument = home)");
    help_line("pwd", "", "current path and its level");
    help_line("cat", "<item...>", "print items");
    help_line("write", "[-a] <item> <text>", "write text to an item");
    help_line(
        "newcluster",
        "<path...>",
        "create a Cluster (inside a Container)",
    );
    help_line("newpkg", "<path...>", "create a Package (inside a Cluster)");
    help_line("newfldr", "<path...>", "create a Folder (inside a Package)");
    help_line(
        "newfl",
        "<path...>",
        "create an Item (inside a Package or Folder)",
    );
    help_line("rm", "<path...>", "delete an Item or an empty holder");
    help_line("stat", "<path>", "show what a path is");
    help_line("df", "", "Container usage");
    help_line("mount", "", "list Containers and virtual mounts");
    help_line("umount", "<path>", "unmount a Container");
    help_line("lsblk", "", "list disks");
    help_line(
        "format",
        "<name> --yes",
        "format hdb as a Container (erases everything)",
    );

    help_heading("Built-in Text editor");
    help_line("zedit", "[file|.]", "text editor (.txt, .config)");

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

    help_heading("System");
    help_line("tskmngr", "", "task manager (interactive)");

    vga::set_color(HELP_DESC);
    kprintln!("Installed packages can be run by name.");
    vga::set_color(HELP_DEFAULT); // restore the shell's default color
}
