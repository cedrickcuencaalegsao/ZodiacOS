//! Shared types between the kernel, the shell and the desktop.
//! Keep this crate tiny, `no_std`, and dependency-free.
#![no_std]
extern crate alloc;

use alloc::{string::String, vec::Vec};

/// A key press delivered to a terminal.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Char(char),
    Enter,
    Backspace,
    Tab,
    Up,
    Down,
}

/// Where the shell writes its output. Implemented by the desktop's
/// Terminal window (and later by kernel/src/console/terminal.rs).
///
/// Control characters understood by implementors:
///   '\n'   -> new line
///   '\x08' -> erase the previous character on the current line
pub trait Console {
    fn write_str(&mut self, s: &str);
    fn clear(&mut self);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FsError {
    NotFound,
    Exists,
    NotADirectory,
    InvalidPath,
}

impl FsError {
    pub fn msg(self) -> &'static str {
        match self {
            FsError::NotFound => "no such file or directory",
            FsError::Exists => "already exists",
            FsError::NotADirectory => "not a directory",
            FsError::InvalidPath => "invalid path",
        }
    }
}

#[derive(Clone, Debug)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub author: String,
    /// Seconds since boot.
    pub created: u64,
    pub modified: u64,
}

/// The file operations the shell needs. Backed by a RAM filesystem now,
/// and by syscalls into the kernel VFS later.
pub trait Fs {
    /// Current time in seconds since boot.
    fn now(&self) -> u64;
    fn is_dir(&self, path: &str) -> bool;
    fn new_file(&mut self, path: &str, author: &str) -> Result<(), FsError>;
    fn new_dir(&mut self, path: &str, author: &str) -> Result<(), FsError>;
    fn list(&self, path: &str) -> Result<Vec<DirEntry>, FsError>;
}

/// What the shell asks the host to do after a command.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    None,
    Shutdown,
    Restart,
}