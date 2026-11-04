use super::layout::{Extent, MAX_EXTENTS};
use alloc::vec::Vec;

/// In-memory Data Allocation Map. It is *derived* state: rebuilt from the
/// object table at mount, so a crash can never leave it disagreeing with the
/// records (worst case: blocks of an interrupted write are simply free again).
pub struct Allocator {
    bits: Vec<u64>,
    total: u32,
}

impl Allocator {
    pub fn new(total: u32) -> Allocator {
        Allocator {
            bits: alloc::vec![0u64; (total as usize + 63) / 64],
            total,
        }
    }

    fn get(&self, i: u32) -> bool {
        (self.bits[(i / 64) as usize] >> (i % 64)) & 1 == 1
    }

    fn set(&mut self, i: u32, v: bool) {
        let w = &mut self.bits[(i / 64) as usize];
        if v {
            *w |= 1 << (i % 64);
        } else {
            *w &= !(1 << (i % 64));
        }
    }

    /// Marks an existing extent as used. Fails on out-of-range or overlap.
    pub fn mark(&mut self, start: u32, len: u32) -> Result<(), ()> {
        let end = start.checked_add(len).ok_or(())?;
        if end > self.total {
            return Err(());
        }
        for i in start..end {
            if self.get(i) {
                return Err(());
            }
            self.set(i, true);
        }
        Ok(())
    }

    pub fn free(&mut self, start: u32, len: u32) {
        let end = start.saturating_add(len).min(self.total);
        for i in start..end {
            self.set(i, false);
        }
    }

    /// (used blocks, total blocks)
    pub fn counts(&self) -> (u32, u32) {
        let used = self.bits.iter().map(|w| w.count_ones()).sum::<u32>();
        (used, self.total)
    }

    /// First-fit: `need` blocks in at most MAX_EXTENTS runs, or None.
    pub fn alloc(&mut self, need: u32) -> Option<[Extent; MAX_EXTENTS]> {
        let mut out = [Extent::default(); MAX_EXTENTS];
        let (mut n, mut remaining, mut i) = (0usize, need, 0u32);
        while remaining > 0 && i < self.total {
            if self.get(i) {
                i += 1;
                continue;
            }
            if n == MAX_EXTENTS {
                return None; // too fragmented for one record
            }
            let start = i;
            let mut len = 0;
            while i < self.total && !self.get(i) && len < remaining {
                len += 1;
                i += 1;
            }
            out[n] = Extent { start, len };
            n += 1;
            remaining -= len;
        }
        if remaining > 0 {
            return None;
        }
        for e in out.iter().take(n) {
            for b in e.start..e.start + e.len {
                self.set(b, true);
            }
        }
        Some(out)
    }
}
