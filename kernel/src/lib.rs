#![no_std]
#![no_main]
#![cfg_attr(target_arch = "x86_64", feature(abi_x86_interrupt))]

extern crate alloc;

// ---- Architecture-neutral ----
mod arch;
mod base;
mod security;

// ---- Shared modules. Hardware that only exists on x86 (VGA, PS/2, ATA, PIC/PIT, BIOS
// memory map) is gated inside each module, not here. ----
mod console;
mod drivers;
mod filesystem;
mod interrupts;
mod legacy;
mod memory;
mod panic;
mod process;
mod scheduler;
mod vga;

// ---- Transition aliases: old module paths keep working while files migrate ----
#[allow(unused_imports)]
pub(crate) use {
    base::sync,
    drivers::{keyboard, serial},
    legacy::package,
    memory::virtual_memory as paging,
    memory::{heap, physical as pmm},
    security::permissions as user,
};

// Genuinely x86 hardware: no aarch64 equivalent of these names
#[cfg(target_arch = "x86_64")]
#[allow(unused_imports)]
pub(crate) use {arch::current::interrupts as idt, drivers::rtc as time, drivers::storage::ata};

// Userspace (desktop) is disabled while it is being rebuilt.
// Boot goes: login -> terminal. Re-enable by restoring the desktop loop
// (see the note at the bottom of kernel_main).

/// Boot sequence shared by x86_64 and AArch64.
const INIT_STEPS: usize = 6; // memory, drivers, fs, interrupts, scheduler, process

pub fn kernel_main() -> ! {
    if let Err(e) = memory::init() {
        panic!("memory init failed: {:?}", e);
    }
    memory::selftest();
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
    process::services::start_managers();
    console::splash::hold(1500);
    drivers::network::start();
    console::splash::step(); // 4
    console::splash::hold(500);
    scheduler::init();
    console::splash::step(); // 5
    console::splash::hold(500);
    process::init();
    console::splash::step(); // 6

    interrupts::enable();
    console::splash::hold(1500);
    console::splash::finish(); // fill to 100%, flash, fade, clear
    log_info!("ZodiacOS kernel started");

    // Login, then straight to the terminal (no desktop for now).
    // Accounts are loaded inside console::login::run() (first call only).
    log_info!("starting login");
    let session = console::auth::run();

    // First-run only: show the recovery code once, then wait for a key.
    // Deliberately printed, not logged: log output can reach serial/log buffers.
    if let Some(code) = console::auth::take_recovery_code() {
        vga::clear();
        kprintln!("Recovery code (write it down, shown once):\n");
        kprintln!("    {}\n", code);
        kprintln!("Press any key to continue.");
        keyboard::flush();
        keyboard::wait_char();
        vga::clear();
    }

    log_info!("user '{}' logged in (uid {})", session.name, session.uid);

    vga::clear();
    kprintln!("ZodiacOS 0.1.0");
    kprintln!("Type 'help' for a list of commands.\n");
    console::terminal::run();
}