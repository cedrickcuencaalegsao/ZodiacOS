//! Bridges ZodiacFS objects to the VFS `Inode` trait, so the existing
//! vfs/mount/file layers work unchanged. The VFS path under a mount point is
//! read as  <cluster>/<package>/<folder>/<item>  (or <package>/<item>).

use super::super::inode::{Class, DirEntry, FileType, Inode, Stat};
use super::container::{Container, ROOT};
use super::layout::Kind;
use crate::base::sync::Mutex;
use alloc::{string::String, sync::Arc, vec::Vec};
use errors::{Error, Result};

pub struct ZNode {
    pub(super) fs: Arc<Mutex<Container>>,
    pub(super) id: u64,
}

impl ZNode {
    pub fn root(fs: Arc<Mutex<Container>>) -> Arc<dyn Inode> {
        Arc::new(ZNode { fs, id: ROOT })
    }

    fn child(&self, id: u64) -> Arc<dyn Inode> {
        Arc::new(ZNode {
            fs: self.fs.clone(),
            id,
        })
    }
}

fn file_type(k: Kind) -> FileType {
    if k == Kind::Item {
        FileType::File
    } else {
        FileType::Directory
    }
}

impl Inode for ZNode {
    fn stat(&self) -> Result<Stat> {
        self.fs.lock().stat(self.id)
    }

    fn class(&self) -> Class {
        match self.fs.lock().kind_of(self.id) {
            Some(Kind::Root) => Class::Container,
            Some(Kind::Cluster) => Class::Cluster,
            Some(Kind::Package) => Class::Package,
            Some(Kind::Folder) => Class::Folder,
            _ => Class::Item,
        }
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        self.fs.lock().read(self.id, offset, buf)
    }

    fn write_at(&self, offset: u64, data: &[u8]) -> Result<usize> {
        self.fs.lock().write(self.id, offset, data)
    }

    fn truncate(&self, size: u64) -> Result<()> {
        self.fs.lock().truncate(self.id, size)
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        let id = self.fs.lock().lookup(self.id, name)?;
        Ok(self.child(id))
    }

    /// `mkdir` creates whatever container level sits below this node
    /// (Container -> Cluster -> Package -> Folder); a file is always an Item.
    fn create(&self, name: &str, kind: FileType, mode: u16) -> Result<Arc<dyn Inode>> {
        let mut c = self.fs.lock();
        let new_kind = match kind {
            FileType::File => Kind::Item,
            FileType::Directory => match c.kind_of(self.id) {
                Some(Kind::Root) => Kind::Cluster,
                Some(Kind::Cluster) => Kind::Package,
                Some(Kind::Package) => Kind::Folder,
                Some(Kind::Folder) => return Err(Error::InvalidArgument), // Folders cannot nest
                _ => return Err(Error::NotADirectory),
            },
            FileType::CharDevice => return Err(Error::Unsupported),
        };
        let id = c.create(self.id, name, new_kind, mode & 0o7777)?;
        drop(c);
        Ok(self.child(id))
    }

    fn unlink(&self, name: &str) -> Result<()> {
        self.fs.lock().remove(self.id, name)
    }

    fn label(&self) -> Option<String> {
        Some(String::from(self.fs.lock().label()))
    }

    fn usage(&self) -> Result<(u64, u64)> {
        Ok(self.fs.lock().usage())
    }

    fn read_dir(&self) -> Result<Vec<DirEntry>> {
        let c = self.fs.lock();
        Ok(c.read_dir(self.id)?
            .into_iter()
            .map(|(name, kind, ino)| DirEntry {
                name,
                kind: file_type(kind),
                ino,
            })
            .collect())
    }
}
