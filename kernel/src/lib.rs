#![no_std]
#![no_main]
#![cfg_attr(target_arch = "x86_64", feature(abi_x86_interrupt))]

#[cfg(target_arch = "x86_64")]
extern crate alloc;

// ---- Architecture-neutral ----
mod arch;
mod base;
mod security;

// ---- x86_64 only for now (VGA, PS/2, ATA, PIC/PIT, BIOS memory map) ----
// Each moves behind a trait as the AArch64 port grows.
#[cfg(target_arch = "x86_64")]
mod console;
#[cfg(target_arch = "x86_64")]
mod drivers;
#[cfg(target_arch = "x86_64")]
mod filesystem;
#[cfg(target_arch = "x86_64")]
mod interrupts;
#[cfg(target_arch = "x86_64")]
mod legacy;
#[cfg(target_arch = "x86_64")]
mod memory;
#[cfg(target_arch = "x86_64")]
mod network;
#[cfg(target_arch = "x86_64")]
mod panic;
#[cfg(target_arch = "x86_64")]
mod process;
#[cfg(target_arch = "x86_64")]
mod scheduler;
#[cfg(target_arch = "x86_64")]
mod vga;

// ---- Transition aliases: old module paths keep working while files migrate ----
#[cfg(target_arch = "x86_64")]
#[allow(unused_imports)]
pub(crate) use {
    arch::current::interrupts as idt,
    base::sync,
    drivers::rtc as time,
    drivers::storage::ata,
    drivers::{keyboard, serial},
    filesystem::zdfs as fs,
    legacy::package,
    memory::virtual_memory as paging,
    memory::{heap, physical as pmm},
    security::permissions as user,
};

/// The desktop, built from userspace/desktop for the ZodiacOS user target and embedded
/// in the kernel image for now. Adjust the path to where your build puts the binary.
/// Later: replace with a read of /bin/desktop from ZDFS.
///
/// Not used yet (the desktop currently runs through `console::desktop_host`), so the
/// dead-code warning is silenced until `process::spawn_elf` is wired up.
#[cfg(target_arch = "x86_64")]
#[allow(dead_code)]
// static DESKTOP_ELF: &[u8] = include_bytes!("../../target/x86_64-zodiac-user/release/desktop");

/// Architecture-neutral boot sequence (x86_64 for now; AArch64 has its own
/// `rust_entry` in arch/aarch64/mod.rs until the shared pieces are ported).
const INIT_STEPS: usize = 6; // memory, drivers, fs, interrupts, scheduler, process

pub fn kernel_main() -> ! {
    if let Err(e) = memory::init() {
        panic!("memory init failed: {}", e);
    }
    drivers::init(); // framebuffer comes up here
    console::splash::begin(INIT_STEPS, 2); // memory + drivers already done
    console::splash::hold(1500);
    if let Err(e) = filesystem::init() {
        console::splash::abort();
        log_error!("filesystem init failed: {:?}", e);
        panic!("filesystem not installed correctly");
    }
    console::splash::step(); // 3
    console::splash::hold(500);
    interrupts::init();
    console::splash::step(); // 4
    console::splash::hold(500);
    scheduler::init();
    console::splash::step(); // 5
    console::splash::hold(500);
    process::init();
    console::splash::step(); // 6

    interrupts::enable();
    console::splash::hold(3000);
    console::splash::finish(); // fill to 100%, flash, fade, clear
    log_info!("ZodiacOS kernel started");

    kprintln!("ZodiacOS 0.1.0");
    kprintln!("Type 'help' for a list of commands.\n");
    console::terminal::run();
}

// Login -> desktop, repeated until the desktop can't start. Logging out of the
// desktop returns to the login screen; a desktop error drops to the terminal.
// loop {
//     log_info!("starting login");
//     let session = console::login::run();
//     log_info!("user '{}' logged in (uid {})", session.name, session.uid);

//     // ADAPT: record the current user so permissions/terminal can use it
//     // user::set_current(session.uid, session.gid);

//     log_info!("starting desktop");
//     match console::desktop_host::run() {
//         Ok(()) => {
//             log_info!("user logged out");
//             vga::clear();
//             continue; // back to the login screen
//         }
//         Err(e) => {
//             log_error!("desktop unavailable: {}", e);
//             break; // fall through to the terminal
//         }
//     }
// }

//     vga::clear();
//     kprintln!("ZodiacOS 0.1.0");
//     kprintln!("Type 'help' for a list of commands.\n");
//     console::terminal::run();
// }
