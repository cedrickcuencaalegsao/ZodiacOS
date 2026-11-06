#![allow(dead_code)]
use super::auth::Identity;
use alloc::{string::String, vec::Vec};

pub type SessionId = u64;

/// Hard cap on stacked sessions (login + nested `su`), so a script can't grow it forever.
pub const MAX_SESSIONS: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SessionKind {
    /// Opened by the login screen; the bottom of the stack.
    Login,
    /// Pushed on top of another session (`su`, privilege elevation).
    Elevated,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SessionError {
    TooManySessions,
    NotFound,
    /// Tried to pop the login session with `pop_elevated`; that's a logout.
    NotElevated,
}

#[derive(Clone)]
pub struct Session {
    pub id: SessionId,
    pub kind: SessionKind,
    /// The session this one was opened from (None for the login session).
    pub parent: Option<SessionId>,
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

    pub fn is_elevated(&self) -> bool {
        self.kind == SessionKind::Elevated
    }

    /// Ticks since this session started. Saturates if the clock went backwards.
    pub fn age(&self, now: u64) -> u64 {
        now.saturating_sub(self.started)
    }
}

/// Stack of sessions: the last one is active. One console today;
/// `su` pushes, `exit` pops. Per-TTY sessions can replace this later.
///
/// Invariants:
///   - index 0 (if any) is the `Login` session; everything above it is `Elevated`.
///   - session ids are never reused.
///   - this type does no I/O and no authentication; it only records who is
///     logged in. Authentication and account storage live in `auth`/`accounts`.
pub struct SessionManager {
    sessions: Vec<Session>,
    next_id: SessionId,
}

impl SessionManager {
    pub const fn new() -> Self {
        Self { sessions: Vec::new(), next_id: 1 }
    }

    /// Open a session for an already-authenticated identity.
    /// Empty stack -> login session. Otherwise -> elevated session on top of the active one.
    pub fn open(&mut self, who: Identity, now: u64) -> Result<SessionId, SessionError> {
        if self.sessions.len() >= MAX_SESSIONS {
            return Err(SessionError::TooManySessions);
        }
        let (kind, parent) = match self.sessions.last() {
            None => (SessionKind::Login, None),
            Some(top) => (SessionKind::Elevated, Some(top.id)),
        };
        let id = self.next_id;
        self.next_id += 1;
        self.sessions.push(Session {
            id, kind, parent,
            uid: who.uid, gid: who.gid, name: who.name, home: who.home,
            started: now,
        });
        Ok(id)
    }

    /// `exit` inside an `su`: pop the top session only if it is elevated.
    /// Returns the popped session. Popping the login session is a logout,
    /// so it is refused here and the caller should use `close_all`.
    pub fn pop_elevated(&mut self) -> Result<Session, SessionError> {
        match self.sessions.last() {
            None => Err(SessionError::NotFound),
            Some(s) if !s.is_elevated() => Err(SessionError::NotElevated),
            Some(_) => Ok(self.sessions.pop().unwrap()),
        }
    }

    /// Close a session by id. Anything opened on top of it is closed too,
    /// since an `su` shell must not outlive the session it came from.
    pub fn close(&mut self, id: SessionId) -> bool {
        match self.sessions.iter().position(|s| s.id == id) {
            Some(pos) => {
                self.sessions.truncate(pos);
                true
            }
            None => false,
        }
    }

    /// Full logout: drop every session.
    pub fn close_all(&mut self) {
        self.sessions.clear();
    }

    pub fn active(&self) -> Option<&Session> {
        self.sessions.last()
    }

    pub fn active_id(&self) -> Option<SessionId> {
        self.sessions.last().map(|s| s.id)
    }

    /// The original login session (who actually sat down at the machine).
    pub fn login_session(&self) -> Option<&Session> {
        self.sessions.first()
    }

    pub fn get(&self, id: SessionId) -> Option<&Session> {
        self.sessions.iter().find(|s| s.id == id)
    }

    pub fn depth(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Bottom (login) to top (active).
    pub fn iter(&self) -> impl Iterator<Item = &Session> {
        self.sessions.iter()
    }
}