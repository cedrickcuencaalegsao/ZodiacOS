//! arch/x86_64/mod.rs (not implemented yet)
pub mod cpu;
pub mod interrupts; // IDT and CPU exception handlers (was idt.rs)
pub mod paging;     // page table entries (the arch-neutral API is memory/virtual_memory.rs)
pub mod port_io;
// pub mod gdt;     // added with the user-mode work (memory part 2)

unsafe extern "C" {
    static __bss_start: u8;
    static __bss_end: u8;
}

/// objcopy -O binary doesn't include .bss, and the bootloader doesn't clear it.
unsafe fn zero_bss() {
    unsafe {
        let start = &raw const __bss_start as *mut u8;
        let end = &raw const __bss_end as *const u8;
        core::ptr::write_bytes(start, 0, end as usize - start as usize);
    }
}

/// Entry point: the bootloader jumps here in 64-bit mode.
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    unsafe { zero_bss() };
    crate::drivers::serial::init();
    interrupts::init(); // load the IDT so faults show an error screen
    crate::kernel_main()
}