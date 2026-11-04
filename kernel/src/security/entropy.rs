use core::arch::x86_64::{__cpuid, _rdrand64_step};

#[target_feature(enable = "rdrand")]
unsafe fn rdrand64() -> Option<u64> {
    for _ in 0..10 {
        let mut v = 0u64;
        if _rdrand64_step(&mut v) == 1 {
            return Some(v);
        }
    }
    None
}

pub fn available() -> bool {
    // CPUID.01H:ECX bit 30 = RDRAND
    __cpuid(1).ecx & (1 << 30) != 0
}

/// Fills `buf` with hardware randomness. Returns false if unavailable.
/// Deliberately has no weak fallback: bad salts are worse than a clear failure.
pub fn fill(buf: &mut [u8]) -> bool {
    if !available() {
        return false;
    }
    for chunk in buf.chunks_mut(8) {
        match unsafe { rdrand64() } {
            Some(v) => chunk.copy_from_slice(&v.to_le_bytes()[..chunk.len()]),
            None => return false,
        }
    }
    true
}

#[cfg(not(target_arch = "x86_64"))]
pub fn available() -> bool { false }

#[cfg(not(target_arch = "x86_64"))]
pub fn fill(_: &mut [u8]) -> bool { false }