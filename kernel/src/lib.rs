#![no_std]
#![no_main]
#![cfg_attr(target_arch = "x86_64", feature(abi_x86_interrupt))]

extern crate alloc;

mod arch;
mod assertions;
mod base;
mod console;
mod drivers;
mod filesystem;
mod interrupts;
mod legacy;
mod memory;
mod panic;
mod security;
mod vga;

// ---- Transition aliases: old module paths keep working while files migrate ----
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

/// Architecture-neutral boot sequence. Each architecture's startup code
/// (arch/<name>/mod.rs) does its CPU setup, then calls this.
pub fn kernel_main() -> ! {
    memory::init(); // physical memory, paging, heap
    interrupts::init();
    interrupts::enable();
    log_info!("ZodiacOS kernel started");

    vga::disable_cursor();
    vga::clear();
    console::splash::run(3000);
    log_info!("splash finished");

    vga::clear();
    vga::enable_cursor();
    kprintln!("ZodiacOS 0.1.0");
    kprintln!("Type 'help' for a list of commands.\n");

    log_info!("starting shell");
    console::terminal::run()
}