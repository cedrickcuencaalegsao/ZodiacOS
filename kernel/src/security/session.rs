#![allow(dead_code)]
use super::auth::Identity;
use alloc::{string::String, vec::Vec};

pub type SessionId = u64;

#[derive(Clone)]
pub struct Session {
    pub id: SessionId,
    pub uid: u32,
    pub gid: u32,
    pub name: String,
    pub home: String,
    pub started: u64, // timer ticks at login
}

impl Session {
    pub fn is_root(&self) -> bool {
        self.uid == 0
    }
}

/// Stack of sessions: the last one is active. One console today;
/// `su` pushes, `exit` pops. Per-TTY sessions can replace this later.
pub struct SessionManager {
    sessions: Vec<Session>,
    next_id: SessionId,
}

impl SessionManager {
    pub const fn new() -> Self {
        Self { sessions: Vec::new(), next_id: 1 }
    }

    pub fn open(&mut self, who: Identity, now: u64) -> SessionId {
        let id = self.next_id;
        self.next_id += 1;
        self.sessions.push(Session {
            id, uid: who.uid, gid: who.gid, name: who.name, home: who.home, started: now,
        });
        id
    }

    pub fn close(&mut self, id: SessionId) -> bool {
        let before = self.sessions.len();
        self.sessions.retain(|s| s.id != id);
        self.sessions.len() != before
    }

    pub fn close_all(&mut self) {
        self.sessions.clear();
    }

    pub fn active(&self) -> Option<&Session> {
        self.sessions.last()
    }

    pub fn get(&self, id: SessionId) -> Option<&Session> {
        self.sessions.iter().find(|s| s.id == id)
    }
}