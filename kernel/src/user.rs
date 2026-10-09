use core::sync::atomic::{AtomicU8, Ordering::Relaxed};

pub const MAX: usize = 8;

// Default user is "root"
static NAME: [AtomicU8; MAX] = [
    AtomicU8::new(b'r'),
    AtomicU8::new(b'o'),
    AtomicU8::new(b'o'),
    AtomicU8::new(b't'),
    AtomicU8::new(0),
    AtomicU8::new(0),
    AtomicU8::new(0),
    AtomicU8::new(0),
];

pub fn get() -> [u8; MAX] {
    let mut out = [0u8; MAX];
    for i in 0..MAX {
        out[i] = NAME[i].load(Relaxed);
    }
    out
}

pub fn set(name: &str) -> Result<(), &'static str> {
    if name.is_empty() || name.len() > MAX || !name.bytes().all(|b| b > 0x20 && b < 0x7F) {
        return Err("invalid user name (1-8 printable characters)");
    }
    for i in 0..MAX {
        NAME[i].store(name.as_bytes().get(i).copied().unwrap_or(0), Relaxed);
    }
    Ok(())
}

pub fn as_str(bytes: &[u8]) -> &str {
    let len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    core::str::from_utf8(&bytes[..len]).unwrap_or("?")
}