use crate::arch::inb;
use core::arch::asm;
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering::{Acquire, Relaxed, Release}};

static SHIFT: AtomicBool = AtomicBool::new(false);

// Scancode set 1, indexed by scancode (0x00..=0x39). 0 = no printable char.
const NORMAL: [u8; 58] = [
    0, 27, b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', b'0', b'-', b'=', 8, b'\t',
    b'q', b'w', b'e', b'r', b't', b'y', b'u', b'i', b'o', b'p', b'[', b']', b'\n', 0,
    b'a', b's', b'd', b'f', b'g', b'h', b'j', b'k', b'l', b';', b'\'', b'`', 0, b'\\',
    b'z', b'x', b'c', b'v', b'b', b'n', b'm', b',', b'.', b'/', 0, b'*', 0, b' ',
];

const SHIFTED: [u8; 58] = [
    0, 27, b'!', b'@', b'#', b'$', b'%', b'^', b'&', b'*', b'(', b')', b'_', b'+', 8, b'\t',
    b'Q', b'W', b'E', b'R', b'T', b'Y', b'U', b'I', b'O', b'P', b'{', b'}', b'\n', 0,
    b'A', b'S', b'D', b'F', b'G', b'H', b'J', b'K', b'L', b':', b'"', b'~', 0, b'|',
    b'Z', b'X', b'C', b'V', b'B', b'N', b'M', b'<', b'>', b'?', 0, b'*', 0, b' ',
];

/// Scancode -> character, tracking the shift state.
fn decode(sc: u8) -> Option<u8> {
    match sc {
        0x2A | 0x36 => {
            SHIFT.store(true, Relaxed);
            None
        }
        0xAA | 0xB6 => {
            SHIFT.store(false, Relaxed);
            None
        }
        _ if sc & 0x80 != 0 => None, // key release
        _ => {
            let table = if SHIFT.load(Relaxed) { &SHIFTED } else { &NORMAL };
            let c = *table.get(sc as usize)?;
            if c == 0 { None } else { Some(c) }
        }
    }
}

// ---- Ring buffer: the IRQ handler produces, the shell consumes ---------------
const CAP: usize = 64;
static BUF: [AtomicU8; CAP] = [const { AtomicU8::new(0) }; CAP];
static HEAD: AtomicUsize = AtomicUsize::new(0);
static TAIL: AtomicUsize = AtomicUsize::new(0);

fn push(c: u8) {
    let head = HEAD.load(Relaxed);
    let next = (head + 1) % CAP;
    if next == TAIL.load(Acquire) {
        return; // buffer full, drop the key
    }
    BUF[head].store(c, Relaxed);
    HEAD.store(next, Release);
}

/// Called from the IRQ1 handler.
pub fn handle_irq() {
    let sc = unsafe { inb(0x60) };
    if let Some(c) = decode(sc) {
        push(c);
    }
}

/// Non-blocking: next typed character, if any.
pub fn read_char() -> Option<u8> {
    let tail = TAIL.load(Relaxed);
    if tail == HEAD.load(Acquire) {
        return None;
    }
    let c = BUF[tail].load(Relaxed);
    TAIL.store((tail + 1) % CAP, Release);
    Some(c)
}

/// Blocks until a key is typed. `hlt` idles the CPU until the next interrupt
/// (the timer ticks every 10 ms, so a key is picked up at the latest then).
pub fn wait_char() -> u8 {
    loop {
        if let Some(c) = read_char() {
            return c;
        }
        unsafe { asm!("hlt", options(nomem, nostack)) };
    }
}

/// Reads the hardware directly, for when interrupts are off (the panic screen).
pub fn poll_char() -> Option<u8> {
    if unsafe { inb(0x64) } & 1 == 0 {
        return None;
    }
    decode(unsafe { inb(0x60) })
}

/// Drops anything the controller buffered before interrupts were enabled.
pub fn flush() {
    for _ in 0..32 {
        if unsafe { inb(0x64) } & 1 == 0 {
            break;
        }
        let _ = unsafe { inb(0x60) };
    }
}