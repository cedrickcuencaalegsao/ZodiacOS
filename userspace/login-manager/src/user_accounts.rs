use alloc::{format, string::String, vec::Vec};

#[derive(Clone)]
pub struct UserAccount {
    pub name: String,
    pub salt: [u8; 16],
    pub hash: [u8; 32],
    pub uid: u32,
    pub gid: u32,
    pub home: String,
}

#[derive(Default)]
pub struct AccountDb {
    pub users: Vec<UserAccount>,
    /// Set when something changed and the kernel should persist `serialize()`.
    pub dirty: bool,
}

impl AccountDb {
    pub fn parse(text: &str) -> Self {
        let mut users = Vec::new();
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
            users.push(UserAccount {
                name: f[0].into(),
                salt,
                hash,
                uid,
                gid,
                home: f[5].into(),
            });
        }
        Self { users, dirty: false }
    }

    pub fn serialize(&self) -> String {
        let mut out = String::new();
        for u in &self.users {
            out.push_str(&format!(
                "{}:{}:{}:{}:{}:{}\n",
                u.name, to_hex(&u.salt), to_hex(&u.hash), u.uid, u.gid, u.home
            ));
        }
        out
    }

    pub fn find(&self, name: &str) -> Option<&UserAccount> {
        self.users.iter().find(|u| u.name == name)
    }

    pub fn add(&mut self, account: UserAccount) {
        self.users.push(account);
        self.dirty = true;
    }

    pub fn is_empty(&self) -> bool {
        self.users.is_empty()
    }
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