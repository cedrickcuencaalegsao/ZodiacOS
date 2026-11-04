#![allow(dead_code)]
use super::{entropy, permissions::MAX as MAX_NAME};
use alloc::{format, string::String, vec::Vec};
use pbkdf2::pbkdf2_hmac;
use sha2::Sha256;

#[cfg(debug_assertions)]
const ROUNDS: u32 = 4096; // QEMU on Apple Silicon is slow; keep debug light
#[cfg(not(debug_assertions))]
const ROUNDS: u32 = 100_000;

const MIN_PASSWORD: usize = 8;
const DUMMY_SALT: [u8; 16] = [0; 16];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthError {
    InvalidCredentials,
    AlreadySetUp,
    WeakPassword,
    InvalidName,
    UserExists,
    NoSuchUser,
    PermissionDenied,
    NoEntropy,
}

/// Who an account is. Returned on successful authentication.
#[derive(Clone)]
pub struct Identity {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub home: String,
}

pub struct UserInfo {
    pub name: String,
    pub uid: u32,
}

struct Secret {
    salt: [u8; 16],
    hash: [u8; 32],
}

struct Account {
    name: String,
    pw: Secret,
    recovery: Option<Secret>,
    uid: u32,
    gid: u32,
    home: String,
}

impl Account {
    fn identity(&self) -> Identity {
        Identity { name: self.name.clone(), uid: self.uid, gid: self.gid, home: self.home.clone() }
    }
}

/// File format, one user per line:
/// name:salt:hash:uid:gid:home:recovery_salt:recovery_hash
/// (the last two are empty when no recovery code exists; 6-field lines still load)
pub struct AuthService {
    accounts: Vec<Account>,
    dirty: bool,
}

impl AuthService {
    pub const fn new() -> Self {
        Self { accounts: Vec::new(), dirty: false }
    }

    pub fn load(file: Option<Vec<u8>>) -> Self {
        let text = file.and_then(|b| String::from_utf8(b).ok()).unwrap_or_default();
        let mut accounts = Vec::new();
        for line in text.lines() {
            let f: Vec<&str> = line.trim().split(':').collect();
            if f.len() != 6 && f.len() != 8 {
                continue;
            }
            let (Some(salt), Some(hash)) = (from_hex::<16>(f[1]), from_hex::<32>(f[2])) else { continue };
            let (Ok(uid), Ok(gid)) = (f[3].parse(), f[4].parse()) else { continue };
            let recovery = if f.len() == 8 {
                match (from_hex::<16>(f[6]), from_hex::<32>(f[7])) {
                    (Some(salt), Some(hash)) => Some(Secret { salt, hash }),
                    _ => None,
                }
            } else {
                None
            };
            accounts.push(Account {
                name: f[0].into(),
                pw: Secret { salt, hash },
                recovery,
                uid,
                gid,
                home: f[5].into(),
            });
        }
        Self { accounts, dirty: false }
    }

    pub fn serialize(&self) -> String {
        let mut out = String::new();
        for a in &self.accounts {
            let (rs, rh) = match &a.recovery {
                Some(r) => (to_hex(&r.salt), to_hex(&r.hash)),
                None => (String::new(), String::new()),
            };
            out.push_str(&format!(
                "{}:{}:{}:{}:{}:{}:{}:{}\n",
                a.name, to_hex(&a.pw.salt), to_hex(&a.pw.hash), a.uid, a.gid, a.home, rs, rh
            ));
        }
        out
    }

    pub fn dirty(&self) -> bool { self.dirty }
    pub fn mark_clean(&mut self) { self.dirty = false; }
    pub fn has_accounts(&self) -> bool { !self.accounts.is_empty() }

    pub fn list_users(&self) -> Vec<UserInfo> {
        self.accounts.iter().map(|a| UserInfo { name: a.name.clone(), uid: a.uid }).collect()
    }

    /// First-run setup. Returns the root recovery code (show it once).
    pub fn create_root(&mut self, password: &str) -> Result<String, AuthError> {
        if self.has_accounts() {
            return Err(AuthError::AlreadySetUp);
        }
        check_password(password)?;
        let (account, code) = build_account("root", password, 0, 0, "/root".into())?;
        self.accounts.push(account);
        self.dirty = true;
        Ok(code)
    }

    /// Caller (service layer) must have verified the requester is root.
    /// Returns the new user's recovery code.
    pub fn add_user(&mut self, name: &str, password: &str) -> Result<String, AuthError> {
        if !valid_name(name) {
            return Err(AuthError::InvalidName);
        }
        if self.accounts.iter().any(|a| a.name == name) {
            return Err(AuthError::UserExists);
        }
        check_password(password)?;
        let uid = self.accounts.iter().map(|a| a.uid).max().map_or(1000, |m| m.max(999) + 1);
        let (account, code) = build_account(name, password, uid, uid, format!("/home/{}", name))?;
        self.accounts.push(account);
        self.dirty = true;
        Ok(code)
    }

    pub fn authenticate(&self, name: &str, password: &str) -> Result<Identity, AuthError> {
        // Hash even for unknown users so timing doesn't reveal which exist.
        let account = self.accounts.iter().find(|a| a.name == name);
        let salt = account.map(|a| &a.pw.salt).unwrap_or(&DUMMY_SALT);
        let computed = hash_secret(salt, password.as_bytes());
        match account {
            Some(a) if ct_eq(&computed, &a.pw.hash) => Ok(a.identity()),
            _ => Err(AuthError::InvalidCredentials),
        }
    }

    pub fn change_password(&mut self, name: &str, old: &str, new: &str) -> Result<(), AuthError> {
        check_password(new)?;
        self.authenticate(name, old)?;
        self.set_password(name, new)
    }

    /// Admin reset. Caller must have verified root.
    pub fn admin_reset(&mut self, name: &str, new: &str) -> Result<(), AuthError> {
        check_password(new)?;
        self.set_password(name, new)
    }

    /// Forgot-password path. Consumes the recovery code and returns a fresh one.
    pub fn recover(&mut self, name: &str, code: &str, new: &str) -> Result<String, AuthError> {
        check_password(new)?;
        let code = normalize_code(code);
        let idx = self.accounts.iter().position(|a| a.name == name);
        let ok = match idx.and_then(|i| self.accounts[i].recovery.as_ref()) {
            Some(r) => ct_eq(&hash_secret(&r.salt, code.as_bytes()), &r.hash),
            None => {
                let _ = hash_secret(&DUMMY_SALT, code.as_bytes());
                false
            }
        };
        if !ok {
            return Err(AuthError::InvalidCredentials);
        }
        let (recovery, new_code) = new_recovery()?;
        let pw = new_secret(new.as_bytes())?;
        let a = &mut self.accounts[idx.unwrap()];
        a.pw = pw;
        a.recovery = Some(recovery);
        self.dirty = true;
        Ok(new_code)
    }

    fn set_password(&mut self, name: &str, new: &str) -> Result<(), AuthError> {
        let pw = new_secret(new.as_bytes())?;
        let a = self.accounts.iter_mut().find(|a| a.name == name).ok_or(AuthError::NoSuchUser)?;
        a.pw = pw;
        self.dirty = true;
        Ok(())
    }
}

// ---------- helpers ----------

fn valid_name(name: &str) -> bool {
    let b = name.as_bytes();
    !b.is_empty()
        && b.len() <= MAX_NAME
        && b[0].is_ascii_lowercase()
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_')
}

fn check_password(p: &str) -> Result<(), AuthError> {
    if p.len() < MIN_PASSWORD { Err(AuthError::WeakPassword) } else { Ok(()) }
}

fn random_salt() -> Result<[u8; 16], AuthError> {
    let mut s = [0u8; 16];
    if entropy::fill(&mut s) { Ok(s) } else { Err(AuthError::NoEntropy) }
}

fn new_secret(data: &[u8]) -> Result<Secret, AuthError> {
    let salt = random_salt()?;
    Ok(Secret { hash: hash_secret(&salt, data), salt })
}

fn new_recovery() -> Result<(Secret, String), AuthError> {
    let mut raw = [0u8; 16];
    if !entropy::fill(&mut raw) {
        return Err(AuthError::NoEntropy);
    }
    let code = to_hex(&raw);
    let secret = new_secret(code.as_bytes())?;
    Ok((secret, group_code(&code)))
}

fn build_account(name: &str, pw: &str, uid: u32, gid: u32, home: String) -> Result<(Account, String), AuthError> {
    let pw = new_secret(pw.as_bytes())?;
    let (recovery, code) = new_recovery()?;
    Ok((Account { name: name.into(), pw, recovery: Some(recovery), uid, gid, home }, code))
}

fn group_code(code: &str) -> String {
    let mut s = String::new();
    for (i, c) in code.chars().enumerate() {
        if i > 0 && i % 4 == 0 {
            s.push('-');
        }
        s.push(c);
    }
    s
}

fn normalize_code(code: &str) -> String {
    code.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect()
}

fn hash_secret(salt: &[u8; 16], secret: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    pbkdf2_hmac::<Sha256>(secret, salt, ROUNDS, &mut out);
    out
}

fn ct_eq(a: &[u8; 32], b: &[u8; 32]) -> bool {
    let mut diff = 0u8;
    for i in 0..32 {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

fn to_hex(bytes: &[u8]) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(H[(b >> 4) as usize] as char);
        s.push(H[(b & 15) as usize] as char);
    }
    s
}

fn from_hex<const N: usize>(s: &str) -> Option<[u8; N]> {
    if s.len() != N * 2 {
        return None;
    }
    let mut out = [0u8; N];
    for (i, pair) in s.as_bytes().chunks(2).enumerate() {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        out[i] = (hi * 16 + lo) as u8;
    }
    Some(out)
}