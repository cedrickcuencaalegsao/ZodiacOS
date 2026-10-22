use crate::base::error::error_enum::Error;

impl Error {
    pub fn errno(self) -> i32 {
        match self {
            Error::NotFound => 2,
            Error::AlreadyExists => 17,
            Error::NotADirectory => 20,
            Error::IsADirectory => 21,
            Error::NotEmpty => 39,
            Error::InvalidArgument => 22,
            Error::NameTooLong => 36,
            Error::PermissionDenied => 13,
            Error::NoSpace => 28,
            Error::OutOfMemory => 12,
            Error::Io => 5,
            Error::Busy => 16,
            Error::BadFileDescriptor => 9,
            Error::ReadOnly => 30,
            Error::Unsupported => 38,
            Error::BadAddress => 14,
            Error::InvalidExecutable => 85,
            Error::TooManyFiles => 24,
            Error::TimeOut => 110,
            Error::Unreachable => 113,
        }
    }
}
