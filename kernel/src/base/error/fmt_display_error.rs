use core::fmt;
use crate::base::error::error_enum::Error;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            Error::NotFound => "no such file or directory",
            Error::AlreadyExists => "already exists",
            Error::NotADirectory => "not a directory",
            Error::IsADirectory => "is a directory",
            Error::NotEmpty => "directory not empty",
            Error::InvalidArgument => "invalid argument",
            Error::NameTooLong => "file name too long",
            Error::PermissionDenied => "permission denied",
            Error::NoSpace => "no space left on device",
            Error::OutOfMemory => "out of memory",
            Error::Io => "input/output error",
            Error::Busy => "device or resource busy",
            Error::BadFileDescriptor => "bad file descriptor",
            Error::ReadOnly => "read-only file system",
            Error::Unsupported => "operation not supported",
            Error::BadAddress => "bad address",
            Error::InvalidExecutable => "invalid executable format",
            Error::TooManyFiles => "too many open files in system",
        })
    }
}