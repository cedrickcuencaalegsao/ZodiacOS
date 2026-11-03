use super::inode::{DirEntry, FileType, Inode, Stat};
use crate::{arch, keyboard, vga};
use alloc::{string::String, sync::Arc, vec::Vec};
use errors::{Error, Result};

#[derive(Clone, Copy)]
enum Dev {
    Null,
    Zero,
    Console,
}

const DEVICES: [(&str, Dev, u64); 3] = [
    ("null", Dev::Null, 2),
    ("zero", Dev::Zero, 3),
    ("console", Dev::Console, 4),
];

struct DevRoot;

struct DevNode {
    ino: u64,
    dev: Dev,
}

pub fn new_root() -> Arc<dyn Inode> {
    Arc::new(DevRoot)
}

impl Inode for DevRoot {
    fn stat(&self) -> Result<Stat> {
        Ok(Stat {
            ino: 1,
            kind: FileType::Directory,
            size: DEVICES.len() as u64,
            mode: 0o755,
            uid: 0,
            gid: 0,
            mtime: 0,
            nlink: 2,
        })
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        match DEVICES.iter().find(|d| d.0 == name) {
            Some(&(_, dev, ino)) => Ok(Arc::new(DevNode { ino, dev })),
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
        Ok(DEVICES
            .iter()
            .map(|&(name, _, ino)| DirEntry {
                name: String::from(name),
                kind: FileType::CharDevice,
                ino,
            })
            .collect())
    }
}

/// Reads one line from the keyboard, echoing it as it is typed.
fn console_read(buf: &mut [u8]) -> usize {
    let mut n = 0;
    while n < buf.len() {
        match keyboard::wait_char() {
            b'\n' => {
                buf[n] = b'\n';
                n += 1;
                vga::put_byte(b'\n');
                break;
            }
            8 => {
                if n > 0 {
                    n -= 1;
                    vga::backspace();
                }
            }
            c @ 0x20..=0x7E => {
                buf[n] = c;
                n += 1;
                vga::put_byte(c);
            }
            _ => {}
        }
    }
    n
}

impl Inode for DevNode {
    fn stat(&self) -> Result<Stat> {
        Ok(Stat {
            ino: self.ino,
            kind: FileType::CharDevice,
            size: 0,
            mode: 0o666,
            uid: 0,
            gid: 0,
            mtime: 0,
            nlink: 1,
        })
    }

    fn read_at(&self, _offset: u64, buf: &mut [u8]) -> Result<usize> {
        match self.dev {
            Dev::Null => Ok(0),
            Dev::Zero => {
                buf.fill(0);
                Ok(buf.len())
            }
            Dev::Console => Ok(console_read(buf)),
        }
    }

    fn write_at(&self, _offset: u64, data: &[u8]) -> Result<usize> {
        match self.dev {
            Dev::Null | Dev::Zero => Ok(data.len()),
            Dev::Console => {
                let was_enabled = arch::save_and_disable_interrupts();
                for &b in data {
                    vga::put_byte(b);
                }
                arch::restore_interrupts(was_enabled);
                Ok(data.len())
            }
        }
    }
}
