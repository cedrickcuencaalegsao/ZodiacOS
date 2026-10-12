use sha2::{Digest, Sha256};

use crate::session::Session;
use crate::user_accounts::{AccountDb, UserAccount};
use alloc::string::String;

const ROUNDS: u32 = 4096;

#[derive(Debug, PartialEq, Eq)]
pub enum AuthError {
    /// Deliberately doesn't say whether the user or the password was wrong.
    InvalidCredentials,
}

/// Salted, iterated SHA-256. Fine for a hobby OS; swap in argon2/pbkdf2 later.
pub fn hash_password(salt: &[u8; 16], password: &str) -> [u8; 32] {
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

/// Constant-time comparison so timing doesn't leak how many bytes matched.
fn ct_eq(a: &[u8; 32], b: &[u8; 32]) -> bool {
    let mut diff = 0u8;
    for i in 0..32 {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

pub fn authenticate(db: &AccountDb, name: &str, password: &str) -> Result<Session, AuthError> {
    // Always do the hashing work, even for unknown users, so response time
    // doesn't reveal which usernames exist.
    const DUMMY_SALT: [u8; 16] = [0; 16];
    let account = db.find(name);
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

pub fn make_account(
    name: &str,
    password: &str,
    uid: u32,
    gid: u32,
    salt: [u8; 16],
) -> UserAccount {
    UserAccount {
        name: name.into(),
        salt,
        hash: hash_password(&salt, password),
        uid,
        gid,
        home: if uid == 0 {
            String::from("/root")
        } else {
            alloc::format!("/home/{}", name)
        },
    }
}