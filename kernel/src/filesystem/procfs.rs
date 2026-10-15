use super::inode::{DirEntry, FileType, Inode, Stat};
use super::mount;
use crate::base::error::{Error, Result};
use crate::process::task;
use crate::{heap, interrupts, pmm, scheduler};
use alloc::{format, string::String, sync::Arc, vec::Vec};

type Generator = fn() -> String;

const FILES: [(&str, Generator); 5] = [
    ("uptime", uptime),
    ("meminfo", meminfo),
    ("version", version),
    ("mounts", mounts),
    ("tasks", tasks),
];

fn uptime() -> String {
    let ms = interrupts::uptime_ms();
    format!("{}.{:02}\n", ms / 1000, ms % 1000 / 10)
}

fn meminfo() -> String {
    let (total, free) = pmm::stats();
    format!(
        "MemTotal: {} kB\nMemFree: {} kB\nHeapTotal: {} kB\nHeapUsed: {} kB\n",
        total * 4,
        free * 4,
        heap::HEAP_SIZE / 1024,
        heap::used() / 1024
    )
}

fn version() -> String {
    format!("ZodiacOS {} (x86_64)\n", env!("CARGO_PKG_VERSION"))
}

fn mounts() -> String {
    let mut out = String::new();
    for (path, fstype, device) in mount::list() {
        out.push_str(&format!("{} {} {}\n", device, path, fstype));
    }
    out
}

fn tasks() -> String {
    let mut out = String::from("PID NAME STATE CPU_MS\n");
    for info in scheduler::snapshot().iter().flatten() {
        out.push_str(&format!(
            "{} {} {} {}\n",
            info.pid,
            task::name_str(&info.name),
            info.state.name(),
            info.cpu_ticks * 1000 / interrupts::TIMER_HZ as u64
        ));
    }
    out
}

struct ProcRoot;

struct ProcFile {
    ino: u64,
    generate: Generator,
}

pub fn new_root() -> Arc<dyn Inode> {
    Arc::new(ProcRoot)
}

impl Inode for ProcRoot {
    fn stat(&self) -> Result<Stat> {
        Ok(Stat {
            ino: 1,
            kind: FileType::Directory,
            size: FILES.len() as u64,
            mode: 0o555,
            uid: 0,
            gid: 0,
            mtime: 0,
            nlink: 2,
        })
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        match FILES.iter().enumerate().find(|(_, f)| f.0 == name) {
            Some((i, &(_, generate))) => Ok(Arc::new(ProcFile {
                ino: 10 + i as u64,
                generate,
            })),
            None => Err(Error::NotFound),
        }
    }

    fn create(&self, _name: &str, _kind: FileType, _mode: u16) -> Result<Arc<dyn Inode>> {
        Err(Error::PermissionDenied)
    }

    fn unlink(&self, _name: &str) -> Result<()> {
        Err(Error::PermissionDenied)
    }

    fn read_dir(&self) -> Result<Vec<DirEntry>> {
        Ok(FILES
            .iter()
            .enumerate()
            .map(|(i, f)| DirEntry {
                name: String::from(f.0),
                kind: FileType::File,
                ino: 10 + i as u64,
            })
            .collect())
    }
}

impl Inode for ProcFile {
    fn stat(&self) -> Result<Stat> {
        Ok(Stat {
            ino: self.ino,
            kind: FileType::File,
            size: (self.generate)().len() as u64,
            mode: 0o444,
            uid: 0,
            gid: 0,
            mtime: 0,
            nlink: 1,
        })
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        let text = (self.generate)();
        let bytes = text.as_bytes();
        if offset >= bytes.len() as u64 {
            return Ok(0);
        }
        let n = buf.len().min(bytes.len() - offset as usize);
        buf[..n].copy_from_slice(&bytes[offset as usize..offset as usize + n]);
        Ok(n)
    }
}
