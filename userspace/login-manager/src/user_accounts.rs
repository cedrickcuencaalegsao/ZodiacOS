use alloc::string::String;

/// Display-only view of an account. No salts or hashes ever reach userspace.
#[derive(Clone, Debug)]
pub struct UserInfo {
    pub name: String,
    pub uid: u32,
}