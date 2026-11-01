#![allow(dead_code)]
//! kernel/src/interrupts/timer.rs
//!
//! 8253/8254 PIT, channel 0 -> IRQ0 -> global tick counter.
//! 100 Hz = 10 ms per tick. tskmngr only needs `ticks()` and `HZ`.
//!
//! Prerequisite: the legacy PIC is already remapped so IRQ0 lands on vector 32
//! (if your keyboard IRQ works, it is). If IRQ0 still points at vector 8, the
//! first tick will look like a double fault.

use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};

pub const HZ: u32 = 100;
pub const IRQ0_VECTOR: u8 = 32;

const PIT_HZ: u32 = 1_193_182; // PIT input clock
const PIT_CH0: u16 = 0x40;
const PIT_CMD: u16 = 0x43;
const PIC1_CMD: u16 = 0x20;
const PIC1_DATA: u16 = 0x21;

static TICKS: AtomicU64 = AtomicU64::new(0);

#[inline]
fn outb(port: u16, val: u8) {
    unsafe {
        asm!("out dx, al", in("dx") port, in("al") val, options(nomem, nostack, preserves_flags))
    };
}

#[inline]
fn inb(port: u16) -> u8 {
    let v: u8;
    unsafe {
        asm!("in al, dx", out("al") v, in("dx") port, options(nomem, nostack, preserves_flags))
    };
    v
}

/// Program the PIT for `HZ` and unmask IRQ0 on the master PIC.
/// Call once during boot, after the IDT is loaded and before `sti`.
pub fn init() {
    let div = (PIT_HZ / HZ) as u16; // 11931 -> 100.0007 Hz
    without_interrupts(|| {
        outb(PIT_CMD, 0x34); // channel 0, lobyte/hibyte, mode 2 (rate generator), binary
        outb(PIT_CH0, (div & 0xff) as u8);
        outb(PIT_CH0, (div >> 8) as u8);
        outb(PIC1_DATA, inb(PIC1_DATA) & !0x01); // unmask IRQ0
    });
}

/// Call from the IRQ0 handler (vector 32).
pub fn handle_irq() {
    let now = TICKS.fetch_add(1, Ordering::Relaxed) + 1;
    // EOI *before* the scheduler runs: if tick() context-switches away, this
    // frame may not resume for a long time and IRQ0 would stay blocked.
    outb(PIC1_CMD, 0x20);
    crate::scheduler::tick(now);
}

/// Timer ticks since `init()`.
#[inline]
pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

#[inline]
pub fn ticks_per_sec() -> u64 {
    HZ as u64
}

pub fn uptime_ms() -> u64 {
    ticks() * 1000 / HZ as u64
}

/// Sleep the *current context* (not the task) until the deadline, halting between ticks.
/// Interrupts must be enabled. For real task sleeping use the scheduler (wake_tick).
pub fn spin_sleep_ms(ms: u64) {
    let end = ticks() + (ms * HZ as u64).div_ceil(1000).max(1);
    while ticks() < end {
        unsafe { asm!("hlt", options(nomem, nostack)) };
    }
}

/// Run `f` with interrupts disabled, restoring the previous IF state afterwards.
/// Use around any lock that the timer IRQ also takes (see scheduler::tick).
pub fn without_interrupts<R>(f: impl FnOnce() -> R) -> R {
    let flags: u64;
    unsafe { asm!("pushfq", "pop {}", "cli", out(reg) flags, options(nomem)) };
    let r = f();
    if flags & (1 << 9) != 0 {
        unsafe { asm!("sti", options(nomem, nostack)) };
    }
    r
}
