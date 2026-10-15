use super::inode::{DirEntry, FileType, Inode, Stat};
use super::path::check_name;
use crate::base::error::{Error, Result};
use crate::base::sync::Mutex;
use crate::time;
use alloc::{collections::BTreeMap, string::String, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering::Relaxed};

const MAX_FILE_SIZE: u64 = 64 * 1024 * 1024;

static NEXT_INO: AtomicU64 = AtomicU64::new(1);

struct Inner {
    mode: u16,
    mtime: u32,
    data: Vec<u8>,                            // file contents
    children: BTreeMap<String, Arc<RamNode>>, // directory entries
}

pub struct RamNode {
    ino: u64,
    kind: FileType,
    inner: Mutex<Inner>,
}

impl RamNode {
    fn new(kind: FileType, mode: u16) -> Arc<RamNode> {
        Arc::new(RamNode {
            ino: NEXT_INO.fetch_add(1, Relaxed),
            kind,
            inner: Mutex::new(Inner {
                mode,
                mtime: time::now(),
                data: Vec::new(),
                children: BTreeMap::new(),
            }),
        })
    }

    fn is_dir(&self) -> bool {
        self.kind == FileType::Directory
    }
}

pub fn new_root() -> Arc<dyn Inode> {
    RamNode::new(FileType::Directory, 0o755)
}

impl Inode for RamNode {
    fn stat(&self) -> Result<Stat> {
        let i = self.inner.lock();
        Ok(Stat {
            ino: self.ino,
            kind: self.kind,
            size: if self.is_dir() {
                i.children.len() as u64
            } else {
                i.data.len() as u64
            },
            mode: i.mode,
            uid: 0,
            gid: 0,
            mtime: i.mtime,
            nlink: if self.is_dir() { 2 } else { 1 },
        })
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        if self.is_dir() {
            return Err(Error::IsADirectory);
        }
        let i = self.inner.lock();
        let len = i.data.len() as u64;
        if offset >= len {
            return Ok(0);
        }
        let n = buf.len().min((len - offset) as usize);
        buf[..n].copy_from_slice(&i.data[offset as usize..offset as usize + n]);
        Ok(n)
    }

    fn write_at(&self, offset: u64, data: &[u8]) -> Result<usize> {
        if self.is_dir() {
            return Err(Error::IsADirectory);
        }
        if data.is_empty() {
            return Ok(0);
        }
        let end = offset
            .checked_add(data.len() as u64)
            .ok_or(Error::InvalidArgument)?;
        if end > MAX_FILE_SIZE {
            return Err(Error::NoSpace);
        }
        let end = end as usize;
        let mut i = self.inner.lock();
        if end > i.data.len() {
            let extra = end - i.data.len();
            i.data.try_reserve(extra).map_err(|_| Error::NoSpace)?;
            i.data.resize(end, 0); // a write past the end leaves a hole of zeros
        }
        i.data[offset as usize..end].copy_from_slice(data);
        i.mtime = time::now();
        Ok(data.len())
    }

    fn truncate(&self, size: u64) -> Result<()> {
        if self.is_dir() {
            return Err(Error::IsADirectory);
        }
        if size > MAX_FILE_SIZE {
            return Err(Error::NoSpace);
        }
        let size = size as usize;
        let mut i = self.inner.lock();
        if size > i.data.len() {
            let extra = size - i.data.len();
            i.data.try_reserve(extra).map_err(|_| Error::NoSpace)?;
        }
        i.data.resize(size, 0);
        i.mtime = time::now();
        Ok(())
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        if !self.is_dir() {
            return Err(Error::NotADirectory);
        }
        let i = self.inner.lock();
        match i.children.get(name) {
            Some(node) => Ok(node.clone() as Arc<dyn Inode>),
            None => Err(Error::NotFound),
        }
    }

    fn create(&self, name: &str, kind: FileType, mode: u16) -> Result<Arc<dyn Inode>> {
        if !self.is_dir() {
            return Err(Error::NotADirectory);
        }
        check_name(name)?;
        if kind == FileType::CharDevice {
            return Err(Error::Unsupported);
        }
        let mut i = self.inner.lock();
        if i.children.contains_key(name) {
            return Err(Error::AlreadyExists);
        }
        let node = RamNode::new(kind, mode);
        i.children.insert(String::from(name), node.clone());
        i.mtime = time::now();
        Ok(node as Arc<dyn Inode>)
    }

    fn unlink(&self, name: &str) -> Result<()> {
        if !self.is_dir() {
            return Err(Error::NotADirectory);
        }
        let mut i = self.inner.lock();
        let child = i.children.get(name).ok_or(Error::NotFound)?;
        if child.is_dir() && !child.inner.lock().children.is_empty() {
            return Err(Error::NotEmpty);
        }
        i.children.remove(name);
        i.mtime = time::now();
        Ok(())
    }

    fn read_dir(&self) -> Result<Vec<DirEntry>> {
        if !self.is_dir() {
            return Err(Error::NotADirectory);
        }
        let i = self.inner.lock();
        Ok(i.children
            .iter()
            .map(|(name, node)| DirEntry {
                name: name.clone(),
                kind: node.kind,
                ino: node.ino,
            })
            .collect())
    }
}
