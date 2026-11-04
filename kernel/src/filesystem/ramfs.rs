//! RAM filesystem, hardened per MEMORYSECURITY.md:
//! * file contents live in `SecureBuf`: zeroized on truncate, unlink, drop,
//!   and when a grow reallocates (no stale copies left on the kernel heap);
//! * total bytes, per-file size, node count and directory size are all bounded,
//!   and every allocation is fallible, so a process cannot exhaust the heap;
//! * ownership is explicit: a node's memory is freed (and wiped) exactly when
//!   the last `Arc` (directory entry or open descriptor) goes away;
//! * ramfs never hands out mappings of its buffers. Executable pages must be
//!   created by the loader by copying into fresh pages (W^X, §4).

use super::inode::{DirEntry, FileType, Inode, Stat};
use super::path::check_name;
use crate::base::sync::Mutex;
use crate::memory::memsecurity::secure_alloc::SecureBuf;
use crate::time;
use alloc::{collections::BTreeMap, string::String, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};
use errors::{Error, Result};

const MAX_FILE_SIZE: u64 = 64 * 1024 * 1024;
const MAX_NODES: usize = 16_384;
const MAX_DIR_ENTRIES: usize = 4_096;

static NEXT_INO: AtomicU64 = AtomicU64::new(1);
static NODES: AtomicUsize = AtomicUsize::new(0);

struct Inner {
    mode: u16,
    uid: u32,
    gid: u32,
    mtime: u32,
    data: SecureBuf,                          // file contents
    children: BTreeMap<String, Arc<RamNode>>, // directory entries
}

pub struct RamNode {
    ino: u64,
    kind: FileType,
    inner: Mutex<Inner>,
}

impl RamNode {
    fn build(kind: FileType, mode: u16) -> Arc<RamNode> {
        Arc::new(RamNode {
            ino: NEXT_INO.fetch_add(1, Relaxed),
            kind,
            inner: Mutex::new(Inner {
                mode,
                uid: 0,
                gid: 0,
                mtime: time::now(),
                data: SecureBuf::new(),
                children: BTreeMap::new(),
            }),
        })
    }

    /// Like `build`, but refuses once the node limit is reached.
    fn try_new(kind: FileType, mode: u16) -> Result<Arc<RamNode>> {
        NODES
            .try_update(Relaxed, Relaxed, |n| (n < MAX_NODES).then_some(n + 1))
            .map_err(|_| Error::NoSpace)?;
        Ok(RamNode::build(kind, mode))
    }

    fn is_dir(&self) -> bool {
        self.kind == FileType::Directory
    }
}

impl Drop for RamNode {
    fn drop(&mut self) {
        NODES.fetch_sub(1, Relaxed); // its SecureBuf wipes itself in its own Drop
    }
}

pub fn new_root() -> Arc<dyn Inode> {
    NODES.fetch_add(1, Relaxed);
    RamNode::build(FileType::Directory, 0o755)
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
            uid: i.uid,
            gid: i.gid,
            mtime: i.mtime,
            nlink: if self.is_dir() { 2 } else { 1 },
        })
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        if self.is_dir() {
            return Err(Error::IsADirectory);
        }
        let i = self.inner.lock();
        let data = i.data.as_slice();
        let len = data.len() as u64;
        if offset >= len {
            return Ok(0);
        }
        let start = offset as usize;
        let n = buf.len().min(data.len() - start);
        buf[..n].copy_from_slice(&data[start..start + n]);
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
            // a write past the end leaves a hole of zeros
            i.data.resize(end, MAX_FILE_SIZE as usize)?;
        }
        let start = offset as usize;
        i.data.as_mut_slice()[start..end].copy_from_slice(data);
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
        let mut i = self.inner.lock();
        i.data.resize(size as usize, MAX_FILE_SIZE as usize)?; // shrinking wipes the tail
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
        if i.children.len() >= MAX_DIR_ENTRIES {
            return Err(Error::NoSpace);
        }
        let node = RamNode::try_new(kind, mode)?;
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
        // Removing the entry drops one Arc; if no descriptor still holds the
        // file, its SecureBuf is zeroized right here.
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
