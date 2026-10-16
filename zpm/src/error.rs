use std::io;
use std::path::Path;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, ZpmError>;

#[derive(Debug, Error)]
pub enum ZpmError {
    #[error("configuration error: {0}")]
    Config(String),
    #[error("invalid manifest: {0}")]
    Manifest(String),
    #[error("invalid package identifier '{0}'")]
    InvalidId(String),
    #[error("invalid version: {0}")]
    InvalidVersion(String),
    #[error("package '{0}' was not found in any configured repository")]
    NotFound(String),
    #[error("package '{id}' exists in several repositories ({repos}); refusing to choose one")]
    Ambiguous { id: String, repos: String },
    #[error("network error: {0}")]
    Network(String),
    #[error("repository error: {0}")]
    Repository(String),
    #[error("insecure repository url '{0}': use https (plain http is only allowed for localhost)")]
    InsecureRepository(String),
    #[error("checksum mismatch for {what}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        what: String,
        expected: String,
        actual: String,
    },
    #[error("signature verification failed: {0}")]
    Signature(String),
    #[error("download interrupted: expected {expected} bytes, received {received}")]
    Interrupted { expected: u64, received: u64 },
    #[error("download exceeds the size limit of {0} bytes")]
    TooLarge(u64),
    #[error("unsafe archive: {0}")]
    UnsafeArchive(String),
    #[error("incompatible package: {0}")]
    Incompatible(String),
    #[error("dependency resolution failed: {0}")]
    Resolution(String),
    #[error("package '{0}' is already installed (use `zpm upgrade` to update it)")]
    AlreadyInstalled(String),
    #[error("package '{0}' is not installed")]
    NotInstalled(String),
    #[error("package '{id}' is required by: {dependents} (use --force to remove anyway)")]
    RequiredBy { id: String, dependents: String },
    #[error("verification failed: {0}")]
    VerifyFailed(String),
    #[error("not enough disk space")]
    StorageFull,
    #[error("transaction failed: {0}")]
    Transaction(String),
    #[error("database error: {0}")]
    Database(String),
    #[error("i/o error: {0}")]
    Io(String),
    #[error("aborted")]
    Aborted,
}

impl ZpmError {
    pub fn io(path: impl AsRef<Path>, err: io::Error) -> Self {
        // ENOSPC on unix, ERROR_DISK_FULL on windows.
        if matches!(err.raw_os_error(), Some(28) | Some(112)) {
            return ZpmError::StorageFull;
        }
        let path = path.as_ref();
        if path.as_os_str().is_empty() {
            ZpmError::Io(err.to_string())
        } else {
            ZpmError::Io(format!("{}: {}", path.display(), err))
        }
    }

    /// Process exit code: 1 general, 3 network, 4 security, 5 resolution,
    /// 6 not found, 7 filesystem/transaction.
    pub fn exit_code(&self) -> u8 {
        use ZpmError::*;
        match self {
            Network(_) | Repository(_) | Interrupted { .. } | TooLarge(_) => 3,
            InsecureRepository(_)
            | ChecksumMismatch { .. }
            | Signature(_)
            | UnsafeArchive(_)
            | VerifyFailed(_) => 4,
            Incompatible(_) | Resolution(_) | Ambiguous { .. } | RequiredBy { .. } => 5,
            NotFound(_) | NotInstalled(_) => 6,
            StorageFull | Transaction(_) | Database(_) | Io(_) => 7,
            _ => 1,
        }
    }
}

impl From<io::Error> for ZpmError {
    fn from(err: io::Error) -> Self {
        ZpmError::io("", err)
    }
}