#![allow(dead_code)]
use super::auth::{AuthError, AuthService, UserInfo};
use super::session::{Session, SessionId, SessionManager};
use super::permissions;
use alloc::{string::String, vec::Vec};
use spin::Mutex;

static AUTH: Mutex<AuthService> = Mutex::new(AuthService::new());
static SESSIONS: Mutex<SessionManager> = Mutex::new(SessionManager::new());
// Lock order: never hold both at once.

/// Call at boot with the accounts file contents (None until ZodiacFS is up).
pub fn init(file: Option<Vec<u8>>) {
    *AUTH.lock() = AuthService::load(file);
}

pub fn needs_setup() -> bool {
    !AUTH.lock().has_accounts()
}

/// First boot: create root. Returns the recovery code, which must be shown once.
pub fn first_run_setup(password: &str) -> Result<String, AuthError> {
    AUTH.lock().create_root(password)
}

pub fn login(name: &str, password: &str, now: u64) -> Result<SessionId, AuthError> {
    let who = AUTH.lock().authenticate(name, password)?;
    let id = SESSIONS
        .lock()
        .open(who, now)
        .map_err(|_| AuthError::SessionLimit)?;
    sync_prompt_name();
    Ok(id)
}

pub fn logout() {
    let mut s = SESSIONS.lock();
    if let Some(id) = s.active().map(|x| x.id) {
        s.close(id);
    }
    drop(s);
    sync_prompt_name();
}

pub fn whoami() -> Option<Session> {
    SESSIONS.lock().active().cloned()
}

fn require_root() -> Result<(), AuthError> {
    match SESSIONS.lock().active() {
        Some(s) if s.is_root() => Ok(()),
        _ => Err(AuthError::PermissionDenied),
    }
}

/// Root only. Returns the new user's recovery code.
pub fn add_user(name: &str, password: &str) -> Result<String, AuthError> {
    require_root()?;
    AUTH.lock().add_user(name, password)
}

/// Active user changes their own password.
pub fn change_password(old: &str, new: &str) -> Result<(), AuthError> {
    let name = whoami().ok_or(AuthError::PermissionDenied)?.name;
    AUTH.lock().change_password(&name, old, new)
}

/// Root only. Resets another user's password without knowing the old one.
pub fn reset_password(target: &str, new: &str) -> Result<(), AuthError> {
    require_root()?;
    AUTH.lock().admin_reset(target, new)
}

/// Forgot password: no session needed. Returns a replacement recovery code.
pub fn recover(name: &str, code: &str, new: &str) -> Result<String, AuthError> {
    AUTH.lock().recover(name, code, new)
}

pub fn list_users() -> Vec<UserInfo> {
    AUTH.lock().list_users()
}

/// Persistence hook: returns the file contents to write when accounts changed.
/// Call after any mutation once ZodiacFS can store e.g. System/Config/accounts.
pub fn take_pending_save() -> Option<String> {
    let mut a = AUTH.lock();
    if a.dirty() {
        a.mark_clean();
        Some(a.serialize())
    } else {
        None
    }
}

fn sync_prompt_name() {
    if let Some(s) = SESSIONS.lock().active() {
        let _ = permissions::set(&s.name); // keeps the shell prompt in step
    }
}

use super::session::SessionError;

/// `su <name>`: authenticate the target and push an elevated session.
pub fn su(name: &str, password: &str, now: u64) -> Result<SessionId, AuthError> {
    if SESSIONS.lock().is_empty() {
        return Err(AuthError::PermissionDenied); // must be logged in first
    }
    let who = AUTH.lock().authenticate(name, password)?;
    let id = SESSIONS.lock().open(who, now).map_err(|_| AuthError::SessionLimit)?;
    sync_prompt_name();
    Ok(id)
}

/// `exit` inside an su: pop one elevated session. Err(NotElevated) means
/// the caller is at the login session, so `exit` should log out instead.
pub fn exit_elevated() -> Result<(), SessionError> {
    SESSIONS.lock().pop_elevated()?;
    sync_prompt_name();
    Ok(())
}

pub fn is_elevated() -> bool {
    SESSIONS.lock().active().map_or(false, |s| s.is_elevated())
}

/// Full logout, including any nested su sessions.
pub fn logout_all() {
    SESSIONS.lock().close_all();
    sync_prompt_name();
}