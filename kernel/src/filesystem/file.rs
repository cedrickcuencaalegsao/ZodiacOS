use super::inode::{DirEntry, FileType, Inode, Stat};
use super::path::resolve_against;
use super::vfs;
use crate::base::sync::Mutex;
use crate::scheduler;
use alloc::{collections::BTreeMap, string::String, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering::Relaxed};
use errors::{Error, Result};

// open() flags: the same values as Linux, so ported programs keep working
pub const O_RDONLY: u32 = 0;
pub const O_WRONLY: u32 = 1;
pub const O_RDWR: u32 = 2;
const O_ACCMODE: u32 = 3;
pub const O_CREAT: u32 = 0x40;
pub const O_EXCL: u32 = 0x80;
pub const O_TRUNC: u32 = 0x200;
pub const O_APPEND: u32 = 0x400;
pub const O_DIRECTORY: u32 = 0x1_0000;

pub const SEEK_SET: u32 = 0;
pub const SEEK_CUR: u32 = 1;
pub const SEEK_END: u32 = 2;

const MAX_FDS: usize = 32;

/// An open file: the inode plus this descriptor's mode and position.
pub struct OpenFile {
    inode: Arc<dyn Inode>,
    flags: u32,
    offset: AtomicU64,
}

struct ProcFiles {
    cwd: String,
    fds: [Option<Arc<OpenFile>>; MAX_FDS],
}

impl ProcFiles {
    fn new() -> ProcFiles {
        let mut files = ProcFiles {
            cwd: String::from("/"),
            fds: core::array::from_fn(|_| None),
        };
        // stdin, stdout and stderr all go to the console (once devfs is mounted)
        if let Ok(console) = vfs::resolve("/dev/console") {
            let file = Arc::new(OpenFile {
                inode: console,
                flags: O_RDWR,
                offset: AtomicU64::new(0),
            });
            for fd in 0..3 {
                files.fds[fd] = Some(file.clone());
            }
        }
        files
    }
}

/// One table per process id, created on first use.
static TABLES: Mutex<BTreeMap<u32, ProcFiles>> = Mutex::new(BTreeMap::new());

fn with_proc<R>(f: impl FnOnce(&mut ProcFiles) -> R) -> R {
    let pid = scheduler::current_pid();
    let mut tables = TABLES.lock();
    let files = tables.entry(pid).or_insert_with(ProcFiles::new);
    f(files)
}

/// Releases a finished process's descriptors (called when the process is reaped).
pub fn process_exit(pid: u32) {
    let table = TABLES.lock().remove(&pid);
    drop(table);
}

fn absolute(path: &str) -> Result<String> {
    let cwd = with_proc(|p| p.cwd.clone());
    resolve_against(&cwd, path)
}

fn get(fd: usize) -> Result<Arc<OpenFile>> {
    with_proc(|p| p.fds.get(fd).and_then(|slot| slot.clone())).ok_or(Error::BadFileDescriptor)
}

// ---- The API ------------------------------------------------------------------

pub fn open(path: &str, flags: u32, mode: u16) -> Result<usize> {
    let abs = absolute(path)?;
    let access = flags & O_ACCMODE;

    let node = match vfs::resolve(&abs) {
        Ok(node) => {
            if flags & O_CREAT != 0 && flags & O_EXCL != 0 {
                return Err(Error::AlreadyExists);
            }
            node
        }
        Err(Error::NotFound) if flags & O_CREAT != 0 => {
            vfs::create(&abs, FileType::File, mode & 0o7777)?
        }
        Err(e) => return Err(e),
    };

    let kind = node.stat()?.kind;
    if kind == FileType::Directory && access != O_RDONLY {
        return Err(Error::IsADirectory);
    }
    if flags & O_DIRECTORY != 0 && kind != FileType::Directory {
        return Err(Error::NotADirectory);
    }
    if flags & O_TRUNC != 0 && access != O_RDONLY && kind == FileType::File {
        node.truncate(0)?;
    }

    let file = Arc::new(OpenFile {
        inode: node,
        flags,
        offset: AtomicU64::new(0),
    });
    with_proc(|p| {
        let slot = p.fds.iter().position(|f| f.is_none())?;
        p.fds[slot] = Some(file);
        Some(slot)
    })
    .ok_or(Error::TooManyFiles)
}

pub fn close(fd: usize) -> Result<()> {
    let file = with_proc(|p| p.fds.get_mut(fd).and_then(|slot| slot.take()));
    file.map(|_| ()).ok_or(Error::BadFileDescriptor)
}

pub fn read(fd: usize, buf: &mut [u8]) -> Result<usize> {
    let file = get(fd)?; // the lock is not held while we read (a console read can block)
    if file.flags & O_ACCMODE == O_WRONLY {
        return Err(Error::BadFileDescriptor);
    }
    let offset = file.offset.load(Relaxed);
    let n = file.inode.read_at(offset, buf)?;
    file.offset.store(offset + n as u64, Relaxed);
    Ok(n)
}

pub fn write(fd: usize, data: &[u8]) -> Result<usize> {
    let file = get(fd)?;
    if file.flags & O_ACCMODE == O_RDONLY {
        return Err(Error::BadFileDescriptor);
    }
    let offset = if file.flags & O_APPEND != 0 {
        file.inode.stat()?.size
    } else {
        file.offset.load(Relaxed)
    };
    let n = file.inode.write_at(offset, data)?;
    file.offset.store(offset + n as u64, Relaxed);
    Ok(n)
}

pub fn lseek(fd: usize, offset: i64, whence: u32) -> Result<u64> {
    let file = get(fd)?;
    let base = match whence {
        SEEK_SET => 0i64,
        SEEK_CUR => file.offset.load(Relaxed) as i64,
        SEEK_END => file.inode.stat()?.size as i64,
        _ => return Err(Error::InvalidArgument),
    };
    let new = base.checked_add(offset).ok_or(Error::InvalidArgument)?;
    if new < 0 {
        return Err(Error::InvalidArgument);
    }
    file.offset.store(new as u64, Relaxed);
    Ok(new as u64)
}

#[allow(dead_code)]
pub fn fstat(fd: usize) -> Result<Stat> {
    get(fd)?.inode.stat()
}

pub fn stat(path: &str) -> Result<Stat> {
    vfs::resolve(&absolute(path)?)?.stat()
}

pub fn mkdir(path: &str, mode: u16) -> Result<()> {
    vfs::create(&absolute(path)?, FileType::Directory, mode & 0o7777).map(|_| ())
}

pub fn unlink(path: &str) -> Result<()> {
    vfs::remove(&absolute(path)?, false)
}

pub fn rmdir(path: &str) -> Result<()> {
    vfs::remove(&absolute(path)?, true)
}

pub fn readdir(path: &str) -> Result<Vec<DirEntry>> {
    vfs::resolve(&absolute(path)?)?.read_dir()
}

pub fn chdir(path: &str) -> Result<()> {
    let abs = absolute(path)?;
    if vfs::resolve(&abs)?.stat()?.kind != FileType::Directory {
        return Err(Error::NotADirectory);
    }
    with_proc(|p| p.cwd = abs);
    Ok(())
}

pub fn getcwd() -> String {
    with_proc(|p| p.cwd.clone())
}
