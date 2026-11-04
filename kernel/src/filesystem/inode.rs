#![allow(dead_code)]
use alloc::{string::String, sync::Arc, vec::Vec};
use errors::{Error, Result};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    File,
    Directory,
    CharDevice,
}

/// What an object *is* in the ZodiacFS model. This is what the user sees.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Container,
    Cluster,
    Package,
    Folder,
    Item,
    Device,
}

impl Class {
    pub fn label(self) -> &'static str {
        match self {
            Class::Container => "Container",
            Class::Cluster => "Cluster",
            Class::Package => "Package",
            Class::Folder => "Folder",
            Class::Item => "Item",
            Class::Device => "Device",
        }
    }

    /// Container, Cluster, Package and Folder hold other objects.
    pub fn is_holder(self) -> bool {
        matches!(
            self,
            Class::Container | Class::Cluster | Class::Package | Class::Folder
        )
    }
}

#[derive(Clone, Copy)]
pub struct Stat {
    pub ino: u64,
    pub kind: FileType,
    pub size: u64,
    pub mode: u16, // permission bits, e.g. 0o644
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

/// A node in some filesystem: an Item, a holder (Container/Cluster/Package/Folder)
/// or a device. Every filesystem implements this; the VFS above it never needs to
/// know which one it is talking to. Defaults say "not supported".
pub trait Inode: Send + Sync {
    fn stat(&self) -> Result<Stat>;

    /// Its ZodiacFS class. The default suits virtual filesystems: a directory
    /// is a Package, a file is an Item, a character device is a Device.
    fn class(&self) -> Class {
        match self.stat().map(|s| s.kind) {
            Ok(FileType::Directory) => Class::Package,
            Ok(FileType::CharDevice) => Class::Device,
            _ => Class::Item,
        }
    }

    /// (used bytes, total bytes) of the Container this node belongs to.
    fn usage(&self) -> Result<(u64, u64)> {
        Err(Error::Unsupported)
    }

    /// The name of the Container this node belongs to, if it has one.
    fn label(&self) -> Option<String> {
        None
    }

    // items and devices
    fn read_at(&self, _offset: u64, _buf: &mut [u8]) -> Result<usize> {
        Err(Error::Unsupported)
    }
    fn write_at(&self, _offset: u64, _data: &[u8]) -> Result<usize> {
        Err(Error::Unsupported)
    }
    fn truncate(&self, _size: u64) -> Result<()> {
        Err(Error::Unsupported)
    }

    // holders
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
