#![allow(dead_code)]

use core::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    NotFound,
    AlreadyExists,
    NotADirectory,
    IsADirectory,
    NotEmpty,
    InvalidArgument,
    NameTooLong,
    PermissionDenied,
    NoSpace,
    OutOfMemory,
    Io,
    Busy,
    BadFileDescriptor,
    ReadOnly,
    Unsupported,
    BadAddress,
    InvalidExecutable,
}

pub type Result<T> = core::result::Result<T, Error>;

impl Error {
    /// Unix errno value, for returning from system calls as `-errno`.
    pub fn errno(self) -> i32 {
        match self {
            Error::NotFound => 2,
            Error::Io => 5,
            Error::BadFileDescriptor => 9,
            Error::OutOfMemory => 12,
            Error::PermissionDenied => 13,
            Error::Busy => 16,
            Error::AlreadyExists => 17,
            Error::NotADirectory => 20,
            Error::IsADirectory => 21,
            Error::InvalidArgument => 22,
            Error::NoSpace => 28,
            Error::ReadOnly => 30,
            Error::NameTooLong => 36,
            Error::Unsupported => 38,
            Error::NotEmpty => 39,
            Error::BadAddress => 14,
            Error::InvalidExecutable => 85,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            Error::NotFound => "no such file or directory",
            Error::AlreadyExists => "already exists",
            Error::NotADirectory => "not a directory",
            Error::IsADirectory => "is a directory",
            Error::NotEmpty => "directory not empty",
            Error::InvalidArgument => "invalid argument",
            Error::NameTooLong => "name too long",
            Error::PermissionDenied => "permission denied",
            Error::NoSpace => "no space left on device",
            Error::OutOfMemory => "out of memory",
            Error::Io => "input/output error",
            Error::Busy => "resource busy",
            Error::BadFileDescriptor => "bad file descriptor",
            Error::ReadOnly => "read-only filesystem",
            Error::Unsupported => "operation not supported",
            Error::BadAddress => "bad address",
            Error::InvalidExecutable => "invalid executable",
        })
    }
}
