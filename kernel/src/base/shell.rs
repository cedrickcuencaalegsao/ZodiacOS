use crate::base::zpm;
use crate::{arch, keyboard, vga};
use crate::{kprint, kprintln};

pub fn run() -> ! {
    let mut buf = [0u8; 128];

    loop {
        vga::set_color(0x0A);
        kprint!("zodiac");
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
        other => {
            // Installed packages work as commands
            if !zpm::launch(other, args) {
                kprintln!("{}: command not found (try 'help')", other);
            }
        }
    }
}

fn help() {
    kprintln!("Built-in commands:");
    kprintln!("  help            show this list");
    kprintln!("  clear           clear the screen");
    kprintln!("  echo <text>     print text");
    kprintln!("  about           show OS version");
    kprintln!("  reboot          restart the machine (alias: restart)");
    kprintln!("  shutdown        power off (alias: poweroff)");
    kprintln!("  zpm <command>   package manager (run 'zpm' for usage)");
    kprintln!("Installed packages can be run by name.");
}