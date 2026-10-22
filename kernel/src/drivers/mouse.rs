//! PS/2 mouse driver (standard 3-byte packets).
//!
//! The mouse sits on the keyboard controller's second port. This driver is *polled*:
//! it leaves IRQ12 disabled and the desktop loop calls `poll()` every few milliseconds,
//! so no IDT or PIC changes are needed. (Moving to IRQ12 later only means calling
//! `feed()` from the handler instead of `poll()`.)
//!
//! Add `pub mod mouse;` to kernel/src/drivers/mod.rs.
#![allow(dead_code)]
use crate::arch::{inb, outb};
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, AtomicUsize, Ordering::Relaxed};

static ENABLED: AtomicBool = AtomicBool::new(false);
static IDX: AtomicUsize = AtomicUsize::new(0); // which byte of the packet is next
static B0: AtomicU8 = AtomicU8::new(0);
static B1: AtomicU8 = AtomicU8::new(0);
static DX: AtomicI32 = AtomicI32::new(0); // motion accumulated since the last take_motion()
static DY: AtomicI32 = AtomicI32::new(0);
static BTN: AtomicU8 = AtomicU8::new(0); // bit0 left, bit1 right, bit2 middle

// ---- keyboard-controller plumbing ---------------------------------------------

fn wait_write() -> bool {
    for _ in 0..100_000 {
        if inb(0x64) & 0x02 == 0 {
            return true;
        }
    }
    false
}

fn wait_read() -> bool {
    for _ in 0..100_000 {
        if inb(0x64) & 0x01 != 0 {
            return true;
        }
    }
    false
}

fn flush_output() {
    for _ in 0..32 {
        if inb(0x64) & 0x01 == 0 {
            break;
        }
        let _ = inb(0x60);
    }
}

/// Sends one command to the mouse itself (not the keyboard) and waits for its ACK (0xFA).
#[allow(unsafe_code)]
fn mouse_cmd(cmd: u8) -> bool {
    if !wait_write() {
        return false;
    }

    outb(0x64, 0xD4);
    // "next byte goes to the mouse"
    if !wait_write() {
        return false;
    }
    outb(0x60, cmd);
    wait_read() && inb(0x60) == 0xFA
}

fn irq_save() -> bool {
    let flags: u64;
    unsafe { core::arch::asm!("pushfq", "pop {}", "cli", out(reg) flags) };
    flags & 0x200 != 0
}

fn irq_restore(were_on: bool) {
    if were_on {
        unsafe { core::arch::asm!("sti") };
    }
}

unsafe fn init_inner() -> bool {
    if !wait_write() {
        return false;
    }
    outb(0x64, 0xA8); // enable the second PS/2 port

    // read the controller configuration byte, change it, write it back
    if !wait_write() {
        return false;
    }
    outb(0x64, 0x20);
    if !wait_read() {
        return false;
    }
    let mut cfg = inb(0x60);
    cfg &= !0x02; // no IRQ12: we poll
    cfg &= !0x20; // second port clock enabled
    if !wait_write() {
        return false;
    }
    outb(0x64, 0x60);
    if !wait_write() {
        return false;
    }
    outb(0x60, cfg);

    flush_output();
    mouse_cmd(0xF6) && mouse_cmd(0xF4) // defaults, then start streaming packets
}

fn reset_state() {
    IDX.store(0, Relaxed);
    DX.store(0, Relaxed);
    DY.store(0, Relaxed);
    BTN.store(0, Relaxed);
}

// ---- public API ---------------------------------------------------------------

/// Initialises the mouse. Returns false if there is none (keyboard-only desktop then).
pub fn init() -> bool {
    reset_state();
    let was_on = irq_save(); // keep the keyboard IRQ from eating the mouse's ACK bytes
    let ok = unsafe { init_inner() };
    irq_restore(was_on);
    ENABLED.store(ok, Relaxed);
    ok
}

/// Stops the mouse streaming. Call this when the desktop ends: an unread mouse byte
/// would otherwise sit in the controller and block the keyboard.
pub fn shutdown() {
    if !ENABLED.swap(false, Relaxed) {
        return;
    }
    let was_on = irq_save();

    let _ = mouse_cmd(0xF5); // disable reporting
    flush_output();

    irq_restore(was_on);
    reset_state();
}

/// Reads whatever mouse bytes the controller has and decodes complete packets.
pub fn poll() {
    if !ENABLED.load(Relaxed) {
        return;
    }
    for _ in 0..64 {
        let mut status = inb(0x64);
        if status & 0x01 == 0 {
            if IDX.load(Relaxed) == 0 {
                break; // nothing pending and not in the middle of a packet
            }
            // mid-packet: the next byte follows within about a millisecond
            let mut got = false;
            for _ in 0..4000 {
                status = inb(0x64);
                if status & 0x01 != 0 {
                    got = true;
                    break;
                }
            }
            if !got {
                IDX.store(0, Relaxed); // lost a byte: resynchronise
                break;
            }
        }
        if status & 0x20 == 0 {
            break; // a keyboard byte: leave it for the keyboard IRQ handler
        }
        feed(inb(0x60));
    }
}

/// Feeds one byte from the mouse into the packet decoder.
pub fn feed(byte: u8) {
    match IDX.load(Relaxed) {
        0 => {
            if byte & 0x08 == 0 {
                return; // bit 3 is always set in the first byte: not a packet start
            }
            B0.store(byte, Relaxed);
            IDX.store(1, Relaxed);
        }
        1 => {
            B1.store(byte, Relaxed);
            IDX.store(2, Relaxed);
        }
        _ => {
            IDX.store(0, Relaxed);
            let flags = B0.load(Relaxed);
            if flags & 0xC0 != 0 {
                return; // x or y overflow: discard the packet
            }
            // 9-bit two's complement: the sign bits live in the flags byte
            let dx = B1.load(Relaxed) as i32 - (((flags as i32) << 4) & 0x100);
            let dy = byte as i32 - (((flags as i32) << 3) & 0x100);
            DX.fetch_add(dx, Relaxed);
            DY.fetch_sub(dy, Relaxed); // PS/2 y grows upward, the screen's grows downward
            BTN.store(flags & 0x07, Relaxed);
        }
    }
}

/// Motion since the last call, in screen coordinates.
pub fn take_motion() -> (i32, i32) {
    (DX.swap(0, Relaxed), DY.swap(0, Relaxed))
}

/// Current button state: bit0 left, bit1 right, bit2 middle.
pub fn buttons() -> u8 {
    BTN.load(Relaxed)
}
