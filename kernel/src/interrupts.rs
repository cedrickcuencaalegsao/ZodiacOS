use crate::arch::outb;
use crate::idt::{self, Frame};
use crate::{keyboard, kassert};
use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering::Relaxed};

pub const TIMER_HZ: u32 = 100; // 10 ms per tick
const PIT_BASE_HZ: u32 = 1_193_182;
const IRQ_BASE: u8 = 32; // master PIC vectors 32..39, slave PIC 40..47

static TICKS: AtomicU64 = AtomicU64::new(0);

fn io_wait() {
    unsafe { outb(0x80, 0) }; // a write to an unused port gives the PIC time to settle
}

/// The BIOS maps IRQs onto vectors 8..15, which collide with CPU exceptions
/// (IRQ0 would look like a double fault). Move them to 32..47.
fn remap_pic() {
    unsafe {
        outb(0x20, 0x11); io_wait(); // start init (cascade mode)
        outb(0xA0, 0x11); io_wait();
        outb(0x21, IRQ_BASE); io_wait(); // master vector offset
        outb(0xA1, IRQ_BASE + 8); io_wait(); // slave vector offset
        outb(0x21, 0x04); io_wait(); // slave sits on master IRQ2
        outb(0xA1, 0x02); io_wait();
        outb(0x21, 0x01); io_wait(); // 8086 mode
        outb(0xA1, 0x01); io_wait();

        outb(0x21, 0xFC); // unmask IRQ0 (timer) and IRQ1 (keyboard) only
        outb(0xA1, 0xFF);
    }
}

fn start_timer(hz: u32) {
    let divisor = (PIT_BASE_HZ / hz) as u16;
    unsafe {
        outb(0x43, 0x36); // channel 0, lobyte/hibyte, mode 3 (square wave)
        outb(0x40, (divisor & 0xFF) as u8);
        outb(0x40, (divisor >> 8) as u8);
    }
}

/// Tell the PIC the interrupt is handled; without this it sends nothing more.
fn eoi(irq: u8) {
    unsafe {
        if irq >= 8 {
            outb(0xA0, 0x20);
        }
        outb(0x20, 0x20);
    }
}

extern "x86-interrupt" fn timer_irq(_f: Frame) {
    TICKS.fetch_add(1, Relaxed);
    eoi(0);
}

extern "x86-interrupt" fn keyboard_irq(_f: Frame) {
    keyboard::handle_irq();
    eoi(1);
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

pub fn disable() {
    unsafe { asm!("cli", options(nomem, nostack)) };
}

pub fn enabled() -> bool {
    let flags: u64;
    unsafe { asm!("pushfq", "pop {}", out(reg) flags) };
    flags & (1 << 9) != 0
}

pub fn ticks() -> u64 {
    TICKS.load(Relaxed)
}

pub fn uptime_ms() -> u64 {
    ticks() * 1000 / TIMER_HZ as u64
}

/// Sleeps with `hlt` so the CPU idles between ticks. Needs interrupts enabled.
pub fn sleep_ms(ms: u64) {
    kassert!(enabled(), "sleep_ms called with interrupts disabled, it would hang forever");
    let target = ticks() + (ms * TIMER_HZ as u64 + 999) / 1000;
    while ticks() < target {
        unsafe { asm!("hlt", options(nomem, nostack)) };
    }
}