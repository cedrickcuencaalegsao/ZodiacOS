#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Copy)]
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
    TooManyFiles,
    TimeOut,
    Unreachable,
}
