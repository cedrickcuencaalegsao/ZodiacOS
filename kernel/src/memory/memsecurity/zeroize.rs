use core::sync::atomic::{Ordering, compiler_fence};

/// Overwrites `buf` with zeros in a way the optimizer may not remove
/// (volatile stores + a compiler fence). MEMORYSECURITY.md §10 / §15.
pub fn zeroize(buf: &mut [u8]) {
    for b in buf.iter_mut() {
        // SAFETY: `b` is a valid, aligned, exclusive reference to a `u8`.
        unsafe { core::ptr::write_volatile(b, 0) };
    }
    compiler_fence(Ordering::SeqCst);
}
