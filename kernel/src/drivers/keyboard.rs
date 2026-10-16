use crate::arch::inb;
use core::arch::asm;
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering::{Acquire, Relaxed, Release}};

pub const KEY_UP: u8 = 0x80;
pub const KEY_DOWN: u8 = 0x81;
pub const KEY_LEFT: u8 = 0x82;
pub const KEY_RIGHT: u8 = 0x83;
pub const KEY_HOME: u8 = 0x84;
pub const KEY_END: u8 = 0x85;
pub const KEY_PAGE_UP: u8 = 0x86;
pub const KEY_PAGE_DOWN: u8 = 0x87;
pub const KEY_DELETE: u8 = 0x88;

static SHIFT: AtomicBool = AtomicBool::new(false);
static CTRL: AtomicBool = AtomicBool::new(false);
static EXTENDED: AtomicBool = AtomicBool::new(false); // an 0xE0 prefix was just seen

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

/// Scancode -> character or special key code, tracking shift, ctrl and 0xE0 prefixes.
fn decode(sc: u8) -> Option<u8> {
    if sc == 0xE0 {
        EXTENDED.store(true, Relaxed);
        return None;
    }
    let extended = EXTENDED.swap(false, Relaxed);
    let released = sc & 0x80 != 0;
    let code = sc & 0x7F;

    if extended {
        if code == 0x1D {
            CTRL.store(!released, Relaxed); // right Ctrl
            return None;
        }
        if released {
            return None;
        }
        return match code {
            0x48 => Some(KEY_UP),
            0x50 => Some(KEY_DOWN),
            0x4B => Some(KEY_LEFT),
            0x4D => Some(KEY_RIGHT),
            0x47 => Some(KEY_HOME),
            0x4F => Some(KEY_END),
            0x49 => Some(KEY_PAGE_UP),
            0x51 => Some(KEY_PAGE_DOWN),
            0x53 => Some(KEY_DELETE),
            _ => None,
        };
    }

    match code {
        0x2A | 0x36 => {
            SHIFT.store(!released, Relaxed);
            None
        }
        0x1D => {
            CTRL.store(!released, Relaxed);
            None
        }
        _ if released => None,
        _ => {
            let table = if SHIFT.load(Relaxed) { &SHIFTED } else { &NORMAL };
            let c = *table.get(code as usize)?;
            if c == 0 {
                return None;
            }
            if CTRL.load(Relaxed) && c.is_ascii_alphabetic() {
                return Some(c & 0x1F); // Ctrl+A = 1 ... Ctrl+L = 12 ... Ctrl+Z = 26
            }
            Some(c)
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

fn pop() -> Option<u8> {
    let tail = TAIL.load(Relaxed);
    if tail == HEAD.load(Acquire) {
        return None;
    }
    let c = BUF[tail].load(Relaxed);
    TAIL.store((tail + 1) % CAP, Release);
    Some(c)
}

/// Called from the IRQ1 handler.
pub fn handle_irq() {
    let sc = inb(0x60) ;
    if let Some(c) = decode(sc) {
        push(c);
    }
}

/// Non-blocking: next typed key. Falls back to the serial port, so you can also
/// type into the terminal that started QEMU (`-serial stdio`).
pub fn read_char() -> Option<u8> {
    if let Some(c) = pop() {
        return Some(c);
    }
    crate::serial::read_byte().map(|b| match b {
        b'\r' => b'\n',
        0x7F => 8, // the terminal's backspace key
        other => other,
    })
}

/// Blocks until a key is typed. `hlt` idles the CPU until the next interrupt.
pub fn wait_char() -> u8 {
    loop {
        if let Some(c) = read_char() {
            return c;
        }
        if crate::scheduler::ready() {
            crate::scheduler::sleep_ms(10); // other threads run in the meantime
        } else {
            unsafe { asm!("hlt", options(nomem, nostack)) };
        }
    }
}

/// Reads the hardware directly, for when interrupts are off (the panic screen).
pub fn poll_char() -> Option<u8> {
    if  inb(0x64)  & 1 == 0 {
        return None;
    }
    decode(inb(0x60) )
}

/// Drops anything the controller buffered before interrupts were enabled.
pub fn flush() {
    for _ in 0..32 {
        if inb(0x64)  & 1 == 0 {
            break;
        }
        let _ = inb(0x60) ;
    }
}