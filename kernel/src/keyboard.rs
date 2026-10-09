use crate::arch::inb;
use core::sync::atomic::{AtomicBool, Ordering::Relaxed};

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

/// Non-blocking: returns a character if a key press is waiting.
pub fn read_char() -> Option<u8> {
    if unsafe { inb(0x64) } & 1 == 0 {
        return None;
    }
    let sc = unsafe { inb(0x60) };

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

pub fn wait_char() -> u8 {
    loop {
        if let Some(c) = read_char() {
            return c;
        }
        core::hint::spin_loop();
    }
}