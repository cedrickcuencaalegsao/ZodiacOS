use super::MAX_TASKS;
use shared::kassert;

pub struct RunQueue {
    buf: [u8; MAX_TASKS],
    head: usize,
    len: usize,
}

impl RunQueue {
    pub const fn new() -> Self {
        RunQueue {
            buf: [0; MAX_TASKS],
            head: 0,
            len: 0,
        }
    }

    pub fn push(&mut self, slot: usize) {
        kassert!(self.len < MAX_TASKS, "run queue overflow");
        self.buf[(self.head + self.len) % MAX_TASKS] = slot as u8;
        self.len += 1;
    }

    pub fn pop(&mut self) -> Option<usize> {
        if self.len == 0 {
            return None;
        }
        let slot = self.buf[self.head] as usize;
        self.head = (self.head + 1) % MAX_TASKS;
        self.len -= 1;
        Some(slot)
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}
