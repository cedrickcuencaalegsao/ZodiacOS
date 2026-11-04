//! `SecureBuf`: an owned byte buffer with explicit ownership, a global quota,
//! and guaranteed zeroization (MEMORYSECURITY.md §2 "secure cleanup", §7, §15).
//!
//! Rules it enforces:
//! * every byte it ever held is zeroed before the memory is released or reused
//!   (shrink, drop, and the *old* buffer when a grow has to reallocate);
//! * total bytes held by all SecureBufs are bounded (`SECURE_POOL_LIMIT`), so
//!   one process cannot exhaust the kernel heap through the filesystem;
//! * all allocation is fallible (`try_reserve_exact`), never aborting.

use super::zeroize::zeroize;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use errors::{Error, Result};

pub const SECURE_POOL_LIMIT: usize = 48 * 1024 * 1024;

static ACCOUNTED: AtomicUsize = AtomicUsize::new(0);

fn charge(bytes: usize) -> Result<()> {
    ACCOUNTED
        .try_update(Relaxed, Relaxed, |cur| {
            cur.checked_add(bytes).filter(|&n| n <= SECURE_POOL_LIMIT)
        })
        .map(|_| ())
        .map_err(|_| Error::NoSpace)
}

fn refund(bytes: usize) {
    ACCOUNTED.fetch_sub(bytes, Relaxed);
}

/// Bytes currently reserved by all SecureBufs (for /proc/meminfo etc.).
#[allow(dead_code)]
pub fn accounted() -> usize {
    ACCOUNTED.load(Relaxed)
}

pub struct SecureBuf {
    data: Vec<u8>,
    charged: usize, // capacity we reserved from the pool
}

impl SecureBuf {
    pub const fn new() -> SecureBuf {
        SecureBuf {
            data: Vec::new(),
            charged: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.data
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.data
    }

    /// Resizes to `new_len` (never above `max_len`). New bytes are zero.
    /// Shrinking zeroizes the discarded tail. Growing past the capacity moves
    /// to a fresh buffer and zeroizes the old one instead of leaving a stale
    /// copy on the heap (what `Vec::resize` would do).
    pub fn resize(&mut self, new_len: usize, max_len: usize) -> Result<()> {
        if new_len > max_len {
            return Err(Error::NoSpace);
        }
        if new_len <= self.data.len() {
            zeroize(&mut self.data[new_len..]);
            self.data.truncate(new_len);
            return Ok(());
        }
        if new_len > self.data.capacity() {
            let target = new_len
                .max(self.data.capacity().saturating_mul(2))
                .min(max_len);
            charge(target)?;
            let mut fresh: Vec<u8> = Vec::new();
            if fresh.try_reserve_exact(target).is_err() {
                refund(target);
                return Err(Error::OutOfMemory);
            }
            fresh.extend_from_slice(&self.data);
            let mut old = core::mem::replace(&mut self.data, fresh);
            zeroize(&mut old);
            refund(self.charged);
            self.charged = target;
        }
        self.data.resize(new_len, 0); // within capacity: no reallocation
        Ok(())
    }
}

impl Drop for SecureBuf {
    fn drop(&mut self) {
        zeroize(&mut self.data);
        refund(self.charged);
    }
}
