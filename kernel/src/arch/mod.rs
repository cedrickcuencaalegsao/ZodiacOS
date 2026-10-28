//! arch/mod.rs (not implemented yet)
//! Architecture layer. Everything CPU-specific lives under arch/<name>/;
//! the rest of the kernel reaches it through `arch::current`.

// src/arch/mod.rs
#[cfg(target_arch = "x86_64")]
pub mod x86_64;
#[cfg(target_arch = "aarch64")]
pub mod aarch64;

#[cfg(target_arch = "x86_64")]
pub use x86_64 as current;
#[cfg(target_arch = "aarch64")]
pub use aarch64 as current;

// Architecture-neutral operations
#[allow(unused_imports)]
#[allow(unused_imports)]
pub use current::cpu::{
    disable_interrupts, enable_interrupts, halt, interrupts_enabled, reboot,
    restore_interrupts, save_and_disable_interrupts, shutdown, sleep_ms,
};

// Port I/O exists only on x86. Drivers that use it are x86-only too.
#[cfg(target_arch = "x86_64")]
pub use current::port_io::{inb, inl, inw, outb, outl, outw};

#[cfg(target_arch = "x86_64")]
pub const BOOT_KSTACK_TOP: u64 = 0x90000;
#[cfg(target_arch = "x86_64")]
pub fn set_kernel_stack(top: u64){
    current::gdt::set_kernel_stack(top);
}

#[cfg(target_arch = "aarch64")]
pub use current::cpu::*;