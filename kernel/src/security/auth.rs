#![allow(dead_code)]
use alloc::{format, string::String, vec::Vec};
use login_manager::session::{AuthError, Session};
use login_manager::user_accounts::UserInfo;
use sha2::{Digest, Sha256};

const ROUNDS: u32 = 4096; // lower while developing in debug builds

struct Account {
    name: String,
    salt: [u8; 16],
    hash: [u8; 32],
    uid: u32,
    gid: u32,
    home: String,
}

/// Account database + verification. Lives only in the kernel.
/// File format, one user per line: name:salt_hex:hash_hex:uid:gid:home
#[derive(Default)]
pub struct AuthService {
    accounts: Vec<Account>,
    dirty: bool,
}

impl AuthService {
    pub fn load(file: Option<Vec<u8>>) -> Self {
        let text = file.and_then(|b| String::from_utf8(b).ok()).unwrap_or_default();
        let mut accounts = Vec::new();
        for line in text.lines() {
            let f: Vec<&str> = line.trim().split(':').collect();
            if f.len() != 6 {
                continue;
            }
            let (Some(salt), Some(hash)) = (from_hex::<16>(f[1]), from_hex::<32>(f[2])) else {
                continue;
            };
            let (Ok(uid), Ok(gid)) = (f[3].parse(), f[4].parse()) else {
                continue;
            };
            accounts.push(Account {
                name: f[0].into(),
                salt,
                hash,
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
            out.push_str(&format!(
                "{}:{}:{}:{}:{}:{}\n",
                a.name, to_hex(&a.salt), to_hex(&a.hash), a.uid, a.gid, a.home
            ));
        }
        out
    }

    pub fn dirty(&self) -> bool {
        self.dirty
    }

    pub fn has_accounts(&self) -> bool {
        !self.accounts.is_empty()
    }

    pub fn list_users(&self) -> Vec<UserInfo> {
        self.accounts
            .iter()
            .map(|a| UserInfo { name: a.name.clone(), uid: a.uid })
            .collect()
    }

    pub fn create_root(&mut self, password: &str, salt: [u8; 16]) -> Result<(), AuthError> {
        if self.has_accounts() {
            return Err(AuthError::AlreadySetUp);
        }
        if password.is_empty() {
            return Err(AuthError::WeakPassword);
        }
        self.accounts.push(Account {
            name: "root".into(),
            salt,
            hash: hash_password(&salt, password),
            uid: 0,
            gid: 0,
            home: "/root".into(),
        });
        self.dirty = true;
        Ok(())
    }

    pub fn authenticate(&self, name: &str, password: &str) -> Result<Session, AuthError> {
        // Hash even for unknown users so timing doesn't reveal which exist.
        const DUMMY_SALT: [u8; 16] = [0; 16];
        let account = self.accounts.iter().find(|a| a.name == name);
        let salt = account.map(|a| &a.salt).unwrap_or(&DUMMY_SALT);
        let computed = hash_password(salt, password);

        match account {
            Some(a) if ct_eq(&computed, &a.hash) => Ok(Session {
                uid: a.uid,
                gid: a.gid,
                name: a.name.clone(),
                home: a.home.clone(),
            }),
            _ => Err(AuthError::InvalidCredentials),
        }
    }
}

fn hash_password(salt: &[u8; 16], password: &str) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(salt);
    h.update(password.as_bytes());
    let mut out: [u8; 32] = h.finalize().into();
    for _ in 0..ROUNDS {
        let mut h = Sha256::new();
        h.update(out);
        h.update(salt);
        out = h.finalize().into();
    }
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