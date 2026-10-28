use core::arch::asm;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};
use shared::kassert;

pub const TIMER_HZ: u32 = 100; // 10 ms per tick

static TICKS: AtomicU64 = AtomicU64::new(0);
static TICK_HOOK: AtomicUsize = AtomicUsize::new(0);

pub fn set_tick_hook(hook: fn(u64)) {
    TICK_HOOK.store(hook as usize, Relaxed);
}

/// Counts one tick. Called first thing in the timer handler.
fn on_tick() -> u64 {
    TICKS.fetch_add(1, Relaxed) + 1
}

/// Runs the scheduler hook. Call it AFTER acknowledging the interrupt, because the
/// hook may switch to another task and not return here for a long time.
fn call_hook(tick: u64) {
    let hook = TICK_HOOK.load(Relaxed);
    if hook != 0 {
        let hook: fn(u64) = unsafe { core::mem::transmute(hook) };
        hook(tick);
    }
}

pub fn ticks() -> u64 {
    TICKS.load(Relaxed)
}

pub fn uptime_ms() -> u64 {
    ticks() * 1000 / TIMER_HZ as u64
}

/// Sleeps with hlt/wfi so the CPU idles between ticks. Needs interrupts enabled.
pub fn sleep_ms(ms: u64) {
    kassert!(
        enabled(),
        "sleep_ms called with interrupts disabled, it would hang forever"
    );
    let target = ticks() + (ms * TIMER_HZ as u64 + 999) / 1000;
    while ticks() < target {
        wait_for_interrupt();
    }
}

#[cfg(target_arch = "x86_64")]
#[allow(unused)]
pub use x86::{disable, enable, enabled, init, wait_for_interrupt};

#[cfg(target_arch = "aarch64")]
#[allow(unused)]
pub use a64::{disable, enable, enabled, init, wait_for_interrupt};

// ===========================================================================
// x86_64: 8259 PIC + 8253 PIT
// Needs `#![feature(abi_x86_interrupt)]` in the crate root, e.g.
//     #![cfg_attr(target_arch = "x86_64", feature(abi_x86_interrupt))]
// ===========================================================================
#[cfg(target_arch = "x86_64")]
mod x86 {
    use super::*;
    use crate::arch::outb;
    use crate::idt::{self, Frame};
    use crate::keyboard;

    const PIT_BASE_HZ: u32 = 1_193_182;
    const IRQ_BASE: u8 = 32; // master PIC vectors 32..39, slave PIC 40..47

    extern "x86-interrupt" fn timer_irq(_f: Frame) {
        let tick = on_tick();
        eoi(0);
        call_hook(tick);
    }

    extern "x86-interrupt" fn keyboard_irq(_f: Frame) {
        keyboard::handle_irq();
        eoi(1);
    }

    fn io_wait() {
        outb(0x80, 0); // a write to an unused port gives the PIC time to settle
    }

    /// The BIOS maps IRQs onto vectors 8..15, which collide with CPU exceptions
    /// (IRQ0 would look like a double fault). Move them to 32..47.
    fn remap_pic() {
        outb(0x20, 0x11);
        io_wait(); // start init (cascade mode)
        outb(0xA0, 0x11);
        io_wait();
        outb(0x21, IRQ_BASE);
        io_wait(); // master vector offset
        outb(0xA1, IRQ_BASE + 8);
        io_wait(); // slave vector offset
        outb(0x21, 0x04);
        io_wait(); // slave sits on master IRQ2
        outb(0xA1, 0x02);
        io_wait();
        outb(0x21, 0x01);
        io_wait(); // 8086 mode
        outb(0xA1, 0x01);
        io_wait();

        outb(0x21, 0xFC); // unmask IRQ0 (timer) and IRQ1 (keyboard) only
        outb(0xA1, 0xFF);
    }

    fn start_timer(hz: u32) {
        let divisor = (PIT_BASE_HZ / hz) as u16;
        outb(0x43, 0x36); // channel 0, lobyte/hibyte, mode 3 (square wave)
        outb(0x40, (divisor & 0xFF) as u8);
        outb(0x40, (divisor >> 8) as u8);
    }

    /// Tell the PIC the interrupt is handled; without this it sends nothing more.
    fn eoi(irq: u8) {
        if irq >= 8 {
            outb(0xA0, 0x20);
        }
        outb(0x20, 0x20);
    }

    pub fn init() {
        remap_pic();
        idt::set_gate(IRQ_BASE, timer_irq as *const () as u64);
        idt::set_gate(IRQ_BASE + 1, keyboard_irq as *const () as u64);
        start_timer(TIMER_HZ);
        keyboard::flush();
    }

    pub fn enable() {
        unsafe { asm!("sti", options(nomem, nostack)) };
    }

    #[allow(dead_code)]
    pub fn disable() {
        unsafe { asm!("cli", options(nomem, nostack)) };
    }

    pub fn enabled() -> bool {
        let flags: u64;
        unsafe { asm!("pushfq", "pop {}", out(reg) flags) };
        flags & (1 << 9) != 0
    }

    pub fn wait_for_interrupt() {
        unsafe { asm!("hlt", options(nomem, nostack)) };
    }
}

// ===========================================================================
// aarch64: GICv2 + ARM generic timer (EL1 physical timer, PPI 30)
// Matches QEMU `-M virt,gic-version=2`. For GICv3 or real boards, change the
// base addresses (and the GICv3 redistributor setup).
// ===========================================================================
#[cfg(target_arch = "aarch64")]
mod a64 {
    use super::*;
    use core::ptr::{read_volatile, write_volatile};

    /// Added to the physical MMIO addresses below. 0 if the MMIO region is
    /// identity-mapped; set it to your direct-map offset in a higher-half kernel.
    const MMIO_VIRT_OFFSET: usize = 0;
    const GICD_BASE: usize = 0x0800_0000 + MMIO_VIRT_OFFSET; // distributor
    const GICC_BASE: usize = 0x0801_0000 + MMIO_VIRT_OFFSET; // CPU interface

    const GICD_CTLR: usize = 0x000;
    const GICD_ISENABLER0: usize = 0x100;
    const GICD_IPRIORITYR: usize = 0x400;
    const GICC_CTLR: usize = 0x000;
    const GICC_PMR: usize = 0x004;
    const GICC_IAR: usize = 0x00C;
    const GICC_EOIR: usize = 0x010;

    const TIMER_INTID: u32 = 30; // EL1 physical timer
    const SPURIOUS: u32 = 1023;

    /// Counter ticks per scheduler tick, set in init().
    static RELOAD: AtomicU64 = AtomicU64::new(0);

    fn reg_write(base: usize, off: usize, val: u32) {
        unsafe { write_volatile((base + off) as *mut u32, val) };
    }

    fn reg_read(base: usize, off: usize) -> u32 {
        unsafe { read_volatile((base + off) as *const u32) }
    }

    fn rearm_timer() {
        let reload = RELOAD.load(Relaxed);
        unsafe { asm!("msr cntp_tval_el0, {}", in(reg) reload, options(nomem, nostack)) };
    }

    /// Called from the "Current EL with SPx, IRQ" vector (and "Lower EL, IRQ" if user
    /// code can be interrupted). The vector stub must save the caller-saved registers
    /// (or the whole frame, if it also context-switches), call this, restore, `eret`.
    #[unsafe(no_mangle)]
    pub extern "C" fn aarch64_irq_handler() {
        let iar = reg_read(GICC_BASE, GICC_IAR);
        let id = iar & 0x3FF;
        if id == SPURIOUS {
            return;
        }
        if id == TIMER_INTID {
            let tick = on_tick();
            rearm_timer();
            reg_write(GICC_BASE, GICC_EOIR, iar);
            call_hook(tick);
        } else {
            // unknown source: just acknowledge it
            reg_write(GICC_BASE, GICC_EOIR, iar);
        }
    }

    pub fn init() {
        // --- GIC: enable the timer PPI and let interrupts through ---
        reg_write(GICD_BASE, GICD_CTLR, 1);
        // priority is one byte per interrupt
        unsafe {
            write_volatile(
                (GICD_BASE + GICD_IPRIORITYR + TIMER_INTID as usize) as *mut u8,
                0x80,
            )
        };
        reg_write(GICD_BASE, GICD_ISENABLER0, 1 << TIMER_INTID);
        reg_write(GICC_BASE, GICC_PMR, 0xFF);
        reg_write(GICC_BASE, GICC_CTLR, 1);

        // --- generic timer ---
        let freq: u64;
        unsafe { asm!("mrs {}, cntfrq_el0", out(reg) freq, options(nomem, nostack)) };
        RELOAD.store(freq / TIMER_HZ as u64, Relaxed);
        rearm_timer();
        // CNTP_CTL_EL0: ENABLE=1, IMASK=0
        unsafe { asm!("msr cntp_ctl_el0, {}", in(reg) 1u64, options(nomem, nostack)) };
        unsafe { asm!("isb", options(nomem, nostack)) };
        // No keyboard IRQ here: there is no PS/2 controller on aarch64.
        // Hook up your UART/virtio-input driver the same way, using its own INTID.
    }

    pub fn enable() {
        unsafe { asm!("msr daifclr, #2", options(nomem, nostack)) }; // unmask IRQ
    }

    #[allow(dead_code)]
    pub fn disable() {
        unsafe { asm!("msr daifset, #2", options(nomem, nostack)) }; // mask IRQ
    }

    pub fn enabled() -> bool {
        let daif: u64;
        unsafe { asm!("mrs {}, daif", out(reg) daif, options(nomem, nostack)) };
        daif & (1 << 7) == 0 // I bit clear = IRQs enabled
    }

    pub fn wait_for_interrupt() {
        unsafe { asm!("wfi", options(nomem, nostack)) };
    }
}
