pub mod cpu;
pub mod exceptions;
pub mod paging;
pub mod context;

use core::fmt;

/// PL011 UART on QEMU's "virt" machine. The firmware already set it up.
const UART0: usize = 0x0900_0000;

pub fn uart_putc(b: u8) {
    unsafe {
        // wait while the transmit FIFO is full (flag register bit 5)
        while core::ptr::read_volatile((UART0 + 0x18) as *const u32) & (1 << 5) != 0 {}
        core::ptr::write_volatile(UART0 as *mut u32, b as u32);
    }
}

pub struct Uart;

impl fmt::Write for Uart {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            if b == b'\n' {
                uart_putc(b'\r');
            }
            uart_putc(b);
        }
        Ok(())
    }
}

#[macro_export]
macro_rules! kprint {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let _ = write!($crate::arch::current::Uart, $($arg)*);
    }};
}

#[macro_export]
macro_rules! kprintln {
    () => ($crate::kprint!("\n"));
    ($($arg:tt)*) => ($crate::kprint!("{}\n", format_args!($($arg)*)));
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    kprintln!("\n*** KERNEL PANIC ***\n{}", info);
    cpu::halt()
}

/// Called from entry.S once the stack, vectors and .bss are ready.
#[unsafe(no_mangle)]
pub extern "C" fn rust_entry(dtb: usize) -> ! {
    kprintln!("ZodiacOS 0.1.0 (aarch64)");
    kprintln!("running at EL{}, device tree at {:#x}", cpu::current_el(), dtb);
    kprintln!("generic timer: {} Hz", cpu::timer_frequency());

    // Prove the exception vectors work: the supervisor call is handled and returns here
    unsafe { core::arch::asm!("svc #7") };
    kprintln!("returned from the exception handler");

    kprintln!("\nAArch64 boot stage reached. No memory manager or shell on this architecture yet.");
    loop {
        cpu::sleep_ms(1000);
        kprint!(".");
    }
}