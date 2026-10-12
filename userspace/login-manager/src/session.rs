use alloc::string::String;

#[derive(Clone, Debug)]
pub struct Session {
    pub uid: u32,
    pub gid: u32,
    pub name: String,
    pub home: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum AuthError {
    InvalidCredentials,
    WeakPassword,
    AlreadySetUp,
}