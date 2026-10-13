#![no_std]
#![no_main]
#![cfg_attr(target_arch = "x86_64", feature(abi_x86_interrupt))]

#[cfg(target_arch = "x86_64")]
extern crate alloc;

// ---- Architecture-neutral ----
mod arch;
mod assertions;
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
mod panic;
#[cfg(target_arch = "x86_64")]
mod vga;
#[cfg(target_arch = "x86_64")]
mod process;
#[cfg(target_arch = "x86_64")]
mod scheduler;

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

/// Architecture-neutral boot sequence (x86_64 for now; AArch64 has its own
/// `rust_entry` in arch/aarch64/mod.rs until the shared pieces are ported).
#[cfg(target_arch = "x86_64")]
pub fn kernel_main() -> ! {
    memory::init();
    drivers::init();
    interrupts::init();
    scheduler::init(); // the boot thread becomes task 0, the timer drives the scheduler
    process::init();   // idle task and the system call gate
    
    interrupts::enable();
    log_info!("ZodiacOS kernel started");

    vga::disable_cursor();
    vga::clear();
    console::splash::run(3000);
    log_info!("splash finished");

    vga::clear();

    log_info!("starting login");
    let session = console::login::run();
    log_info!("user '{}' logged in (uid {})", session.name, session.uid);

    // ADAPT: record the current user so permissions/terminal can use it
    // user::set_current(session.uid, session.gid);

    vga::clear();
    kprintln!("ZodiacOS 0.1.0");
    kprintln!("Type 'help' for a list of commands.\n");

    log_info!("starting shell");
    console::terminal::run();
}