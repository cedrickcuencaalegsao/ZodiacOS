//! Scheduling policy: plain round-robin with a fixed time slice.

/// 5 ticks x 10 ms = 50 ms per turn
pub const QUANTUM_TICKS: u32 = 5;

pub fn new_slice() -> u32 {
    QUANTUM_TICKS
}

/// Called once per timer tick for the running task. True when its slice is used up.
pub fn slice_expired(slice_left: &mut u32) -> bool {
    *slice_left = slice_left.saturating_sub(1);
    *slice_left == 0
}