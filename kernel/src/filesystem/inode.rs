use crate::base::error::{Error, Result};
use alloc::{string::String, sync::Arc, vec::Vec};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    File,
    Directory,
    CharDevice,
}

#[derive(Clone, Copy)]
pub struct Stat {
    pub ino: u64,
    pub kind: FileType,
    pub size: u64,
    pub mode: u16, // Unix permission bits, e.g. 0o644
    pub uid: u32,
    pub gid: u32,
    pub mtime: u32,
    pub nlink: u32,
}

pub struct DirEntry {
    pub name: String,
    pub kind: FileType,
    pub ino: u64,
}

/// A node in some filesystem: a file, a directory or a device. Every filesystem
/// implements this; the VFS above it never needs to know which one it is talking to.
/// Defaults say "not supported", so a filesystem only fills in what applies.
pub trait Inode: Send + Sync {
    fn stat(&self) -> Result<Stat>;

    // files and devices
    fn read_at(&self, _offset: u64, _buf: &mut [u8]) -> Result<usize> {
        Err(Error::Unsupported)
    }
    fn write_at(&self, _offset: u64, _data: &[u8]) -> Result<usize> {
        Err(Error::Unsupported)
    }
    fn truncate(&self, _size: u64) -> Result<()> {
        Err(Error::Unsupported)
    }

    // directories
    fn lookup(&self, _name: &str) -> Result<Arc<dyn Inode>> {
        Err(Error::NotADirectory)
    }
    fn create(&self, _name: &str, _kind: FileType, _mode: u16) -> Result<Arc<dyn Inode>> {
        Err(Error::NotADirectory)
    }
    fn unlink(&self, _name: &str) -> Result<()> {
        Err(Error::NotADirectory)
    }
    fn read_dir(&self) -> Result<Vec<DirEntry>> {
        Err(Error::NotADirectory)
    }
}
